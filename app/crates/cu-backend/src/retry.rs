// SPDX-License-Identifier: GPL-3.0-or-later

use std::future::Future;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::BackendError;
use crate::rng::Rng;

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        ms
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub base_ms: u64,
    pub max_ms: u64,
    pub max_attempts: u32,
}

impl RetryPolicy {
    #[must_use]
    pub const fn s3_default() -> Self {
        Self {
            base_ms: 200,
            max_ms: 30_000,
            max_attempts: 5,
        }
    }

    fn cap_ms(&self, attempt: u32) -> u64 {
        let shift = attempt.min(20);
        self.max_ms.min(self.base_ms.saturating_mul(1u64 << shift))
    }

    fn jittered_delay_ms(&self, attempt: u32, rng: &mut Rng) -> u64 {
        let cap = self.cap_ms(attempt);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let delay = (rng.unit() * (cap as f64 + 1.0)) as u64;
        delay.min(cap)
    }
}

#[derive(Debug, Clone, Copy)]
enum CbState {
    Closed { consecutive_failures: u32 },
    Open { until_ms: i64 },
}

pub struct CircuitBreaker {
    failure_threshold: u32,
    open_ms: i64,
    state: Mutex<CbState>,
}

impl CircuitBreaker {
    #[must_use]
    pub const fn new(failure_threshold: u32, open_ms: i64) -> Self {
        Self {
            failure_threshold,
            open_ms,
            state: Mutex::new(CbState::Closed {
                consecutive_failures: 0,
            }),
        }
    }

    #[must_use]
    pub const fn s3_default() -> Self {
        Self::new(5, 30_000)
    }

    pub fn allow(&self, now_ms: i64) -> bool {
        let mut state = self.state.lock().unwrap();
        match *state {
            CbState::Closed { .. } => true,
            CbState::Open { until_ms } => {
                if now_ms >= until_ms {
                    *state = CbState::Closed {
                        consecutive_failures: 0,
                    };
                    true
                } else {
                    false
                }
            }
        }
    }

    pub(crate) fn record_success(&self) {
        *self.state.lock().unwrap() = CbState::Closed {
            consecutive_failures: 0,
        };
    }

    pub(crate) fn record_failure(&self, now_ms: i64) {
        let mut state = self.state.lock().unwrap();
        let consecutive_failures = match *state {
            CbState::Closed {
                consecutive_failures,
            } => consecutive_failures + 1,
            CbState::Open { .. } => self.failure_threshold,
        };
        *state = if consecutive_failures >= self.failure_threshold {
            CbState::Open {
                until_ms: now_ms + self.open_ms,
            }
        } else {
            CbState::Closed {
                consecutive_failures,
            }
        };
    }
}

pub struct RetryFailure {
    pub error: BackendError,
    pub retry_after_ms: Option<u64>,
}

impl From<BackendError> for RetryFailure {
    fn from(error: BackendError) -> Self {
        Self {
            error,
            retry_after_ms: None,
        }
    }
}

const MAX_HINTED_DELAY_MS: u64 = 120_000;

#[must_use]
pub fn jitter_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

pub async fn retry<T, Fut, S, SFut>(
    policy: &RetryPolicy,
    breaker: &CircuitBreaker,
    clock: &dyn Clock,
    seed: u64,
    sleep: S,
    mut attempt_fn: impl FnMut(u32) -> Fut,
) -> Result<T, BackendError>
where
    Fut: Future<Output = Result<T, BackendError>>,
    S: Fn(u64) -> SFut,
    SFut: Future<Output = ()>,
{
    retry_with_hint(policy, breaker, clock, seed, sleep, |attempt| {
        let fut = attempt_fn(attempt);
        Box::pin(async move { fut.await.map_err(RetryFailure::from) })
    })
    .await
}

pub async fn retry_with_hint<T, Fut, S, SFut>(
    policy: &RetryPolicy,
    breaker: &CircuitBreaker,
    clock: &dyn Clock,
    seed: u64,
    sleep: S,
    mut attempt_fn: impl FnMut(u32) -> Fut,
) -> Result<T, BackendError>
where
    Fut: Future<Output = Result<T, RetryFailure>>,
    S: Fn(u64) -> SFut,
    SFut: Future<Output = ()>,
{
    let mut rng = Rng::new(seed);
    let mut attempt = 0u32;
    loop {
        let now = clock.now_ms();
        if !breaker.allow(now) {
            return Err(BackendError::Coded(cu_msg::Msg::new(
                "BACKEND_CIRCUIT_OPEN",
            )));
        }
        match attempt_fn(attempt).await {
            Ok(v) => {
                breaker.record_success();
                return Ok(v);
            }
            Err(failure) if failure.error.is_transient() && attempt + 1 < policy.max_attempts => {
                breaker.record_failure(clock.now_ms());
                let jitter = policy.jittered_delay_ms(attempt, &mut rng);
                let delay = match failure.retry_after_ms {
                    Some(hint) => jitter.max(hint).min(MAX_HINTED_DELAY_MS),
                    None => jitter,
                };
                sleep(delay).await;
                attempt += 1;
            }
            Err(failure) => {
                if failure.error.is_transient() {
                    breaker.record_failure(clock.now_ms());
                }
                return Err(failure.error);
            }
        }
    }
}
