// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Local, Timelike};
use tokio::sync::watch;
use tokio::time::{Instant, MissedTickBehavior, Sleep, interval, sleep};

use crate::job::{JobError, JobRunner, JobSpec, window_allows};
use crate::status::{JobStatus, Outcome, StatusStore};
use crate::watcher::{self, DirWatcher};

pub const CORRECTNESS_INTERVAL: Duration = Duration::from_secs(6 * 3600);

const WATCHER_DEBOUNCE: Duration = Duration::from_secs(5);

const WATCHER_COOLDOWN: Duration = Duration::from_secs(60);

const BACKOFF_BASE: Duration = Duration::from_secs(60);
const BACKOFF_CAP: Duration = Duration::from_secs(30 * 60);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[must_use]
pub fn effective_interval(configured_secs: u64) -> Duration {
    Duration::from_secs(configured_secs.max(1)).min(CORRECTNESS_INTERVAL)
}

fn next_backoff(current: Duration) -> Duration {
    (current * 2).min(BACKOFF_CAP)
}

enum Attempt {
    Ok,
    PasswordUnavailable,
    Failed,
}

fn local_weekday_and_minute(now: SystemTime) -> (u8, u16) {
    let dt: DateTime<Local> = now.into();
    let weekday = u8::try_from(dt.weekday().num_days_from_monday()).unwrap_or(0);
    let minute_of_day = u16::try_from(dt.hour() * 60 + dt.minute()).unwrap_or(0);
    (weekday, minute_of_day)
}

fn schedule_allows_now(spec: &JobSpec) -> bool {
    if spec.schedule_windows.is_empty() {
        return true;
    }
    let (weekday, minute_of_day) = local_weekday_and_minute(SystemTime::now());
    window_allows(&spec.schedule_windows, weekday, minute_of_day)
}

fn random_delay(max_secs: u32) -> Duration {
    use std::hash::{BuildHasher, Hasher};
    let max_ms = u64::from(max_secs.min(crate::job::MAX_RANDOM_DELAY_SECS)) * 1000;
    if max_ms == 0 {
        return Duration::ZERO;
    }
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(now_ms().unsigned_abs());
    Duration::from_millis(hasher.finish() % (max_ms + 1))
}

async fn update_status_blocking(
    status: &StatusStore,
    name: &str,
    f: impl FnOnce(&mut JobStatus) + Send + 'static,
) {
    let status = status.clone();
    let name = name.to_string();
    let _ = tokio::task::spawn_blocking(move || status.update(&name, f)).await;
}

async fn run_once<R: JobRunner>(spec: &JobSpec, runner: &Arc<R>, status: &StatusStore) -> Attempt {
    if !schedule_allows_now(spec) {
        let now = now_ms();
        update_status_blocking(status, &spec.name, move |s| {
            s.last_outcome = Some(Outcome::OutsideWindow);
            s.updated_at_ms = now;
        })
        .await;
        return Attempt::Ok;
    }

    let delay = random_delay(spec.timing.random_delay_max_secs);
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }

    let started_at = now_ms();
    let result = runner.run_once(spec).await;
    let finished_at = now_ms();

    let (outcome, attempt) = match &result {
        Ok(summary) => (
            Outcome::Ok {
                uploaded: summary.uploaded,
                deleted: summary.deleted,
                renamed: summary.renamed,
                bytes_transferred: summary.bytes_transferred,
                conflicts: summary.conflicts,
                ignored_kept: summary.ignored_kept,
                post_hook: summary.post_hook.clone(),
            },
            Attempt::Ok,
        ),
        Err(JobError::PasswordUnavailable) => {
            (Outcome::PasswordUnavailable, Attempt::PasswordUnavailable)
        }
        Err(JobError::Failed(msg)) => (
            Outcome::Failed {
                message: msg.wire(),
            },
            Attempt::Failed,
        ),
    };

    update_status_blocking(status, &spec.name, move |s| {
        s.last_run_started_at_ms = Some(started_at);
        s.last_run_finished_at_ms = Some(finished_at);
        s.last_outcome = Some(outcome);
        s.updated_at_ms = finished_at;
    })
    .await;

    attempt
}

async fn write_watcher_health(status: &StatusStore, name: &str, watcher: &DirWatcher) {
    let (health, detail) = watcher.health.borrow().clone();
    update_status_blocking(status, name, move |s| s.set_watcher_health(health, &detail)).await;
}

async fn wait_pending(slot: &mut Option<Pin<Box<Sleep>>>) {
    match slot {
        Some(s) => s.await,
        None => std::future::pending::<()>().await,
    }
}

pub async fn run_job<R: JobRunner>(
    spec: JobSpec,
    runner: Arc<R>,
    status: StatusStore,
    shutdown: watch::Receiver<bool>,
) {
    let w = watcher::spawn(&spec.local_root);
    run_job_with_watcher(spec, runner, status, shutdown, w).await;
}

pub async fn run_job_with_watcher<R: JobRunner>(
    spec: JobSpec,
    runner: Arc<R>,
    status: StatusStore,
    mut shutdown: watch::Receiver<bool>,
    mut w: DirWatcher,
) {
    write_watcher_health(&status, &spec.name, &w).await;

    let first_attempt = run_once(&spec, &runner, &status).await;

    let interval_duration = effective_interval(spec.interval_secs);
    let mut timer = interval(interval_duration);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    timer.tick().await;

    let mut debounce: Option<Pin<Box<Sleep>>> = None;
    let mut backoff_delay = BACKOFF_BASE;
    let mut backoff_sleep: Option<Pin<Box<Sleep>>> =
        matches!(first_attempt, Attempt::Failed).then(|| Box::pin(sleep(backoff_delay)));
    let mut last_watcher_run = Instant::now()
        .checked_sub(WATCHER_COOLDOWN)
        .unwrap_or_else(Instant::now);

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            _ = timer.tick() => {
                match run_once(&spec, &runner, &status).await {
                    Attempt::Ok | Attempt::PasswordUnavailable => {
                        backoff_delay = BACKOFF_BASE;
                        backoff_sleep = None;
                    }
                    Attempt::Failed => {
                        backoff_delay = BACKOFF_BASE;
                        backoff_sleep = Some(Box::pin(sleep(backoff_delay)));
                    }
                }
                debounce = None;
            }
            Some(()) = w.wake.recv() => {
                debounce = Some(Box::pin(sleep(WATCHER_DEBOUNCE)));
            }
            () = wait_pending(&mut debounce), if debounce.is_some() => {
                debounce = None;
                if last_watcher_run.elapsed() >= WATCHER_COOLDOWN {
                    last_watcher_run = Instant::now();
                    match run_once(&spec, &runner, &status).await {
                        Attempt::Ok | Attempt::PasswordUnavailable => {
                            backoff_delay = BACKOFF_BASE;
                            backoff_sleep = None;
                        }
                        Attempt::Failed => {
                            backoff_delay = BACKOFF_BASE;
                            backoff_sleep = Some(Box::pin(sleep(backoff_delay)));
                        }
                    }
                    timer.reset();
                }
            }
            () = wait_pending(&mut backoff_sleep), if backoff_sleep.is_some() => {
                backoff_sleep = None;
                match run_once(&spec, &runner, &status).await {
                    Attempt::Ok | Attempt::PasswordUnavailable => {
                        backoff_delay = BACKOFF_BASE;
                    }
                    Attempt::Failed => {
                        backoff_delay = next_backoff(backoff_delay);
                        backoff_sleep = Some(Box::pin(sleep(backoff_delay)));
                    }
                }
            }
            Ok(()) = w.health.changed() => {
                write_watcher_health(&status, &spec.name, &w).await;
            }
        }
    }
}

#[must_use]
pub fn default_status_dir(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("cryptoupload").join("jobs").join("status")
}
