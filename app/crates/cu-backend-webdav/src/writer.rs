// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use cu_backend::{
    BackendError, CircuitBreaker, Clock, RetryFailure, RetryPolicy, StreamWriter, jitter_seed,
    retry_with_hint,
};
use tokio::io::AsyncWriteExt;

use crate::backend::SleepFn;
use crate::errclass::{classify_reqwest, classify_status};

static SPOOL_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn fresh_source_marker() -> [u8; 32] {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&nanos.to_le_bytes());
    out[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    out
}

fn unique_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let counter = SPOOL_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{}-{counter:x}", std::process::id())
}

fn fresh_spool_path(dir: &std::path::Path, token: &str) -> PathBuf {
    static SWEPT: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut swept) = SWEPT.lock()
        && !swept.iter().any(|d| d == dir)
    {
        swept.push(dir.to_path_buf());
        let _ = cu_backend::spool::sweep_stale(
            dir,
            "cu-webdav-",
            ".part",
            cu_backend::spool::STALE_SPOOL_AGE,
        );
    }
    dir.join(format!("cu-webdav-{token}.part"))
}

pub(crate) const TEMP_PREFIX: &str = ".cu-upload-";

fn temp_sibling_url(final_url: &str, token: &str) -> String {
    let split_at = final_url.rfind('/').map_or(0, |i| i + 1);
    let dir_prefix = &final_url[..split_at];
    format!("{dir_prefix}{TEMP_PREFIX}{token}")
}

pub struct WebDavWriter {
    http: reqwest::Client,
    final_url: String,
    temp_url: String,
    username: String,
    password: String,
    exclusive: bool,
    retry_policy: RetryPolicy,
    breaker: Arc<CircuitBreaker>,
    clock: Arc<dyn Clock>,
    sleep: SleepFn,

    spool_path: PathBuf,
    spool_file: Option<tokio::fs::File>,
}

impl WebDavWriter {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        http: reqwest::Client,
        final_url: String,
        username: String,
        password: String,
        exclusive: bool,
        retry_policy: RetryPolicy,
        breaker: Arc<CircuitBreaker>,
        clock: Arc<dyn Clock>,
        sleep: SleepFn,
        spool_dir: &std::path::Path,
    ) -> Self {
        let token = unique_token();
        let temp_url = temp_sibling_url(&final_url, &token);
        Self {
            http,
            final_url,
            temp_url,
            username,
            password,
            exclusive,
            retry_policy,
            breaker,
            clock,
            sleep,
            spool_path: fresh_spool_path(spool_dir, &token),
            spool_file: None,
        }
    }

    async fn ensure_spool_file(&mut self) -> Result<(), BackendError> {
        if self.spool_file.is_some() {
            return Ok(());
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&self.spool_path).await?;
        self.spool_file = Some(file);
        Ok(())
    }

    async fn cleanup_local_spool(&self) {
        let _ = tokio::fs::remove_file(&self.spool_path).await;
    }

    async fn cleanup_remote_temp(&self) {
        let _ = self
            .http
            .delete(&self.temp_url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await;
    }

    async fn put_temp_object(&self) -> Result<(), BackendError> {
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let body = if self.spool_path.exists() {
                    let file = tokio::fs::File::open(&self.spool_path)
                        .await
                        .map_err(|e| RetryFailure::from(BackendError::from(e)))?;
                    reqwest::Body::from(file)
                } else {
                    reqwest::Body::from(Vec::new())
                };
                let resp = self
                    .http
                    .put(&self.temp_url)
                    .basic_auth(&self.username, Some(&self.password))
                    .body(body)
                    .send()
                    .await
                    .map_err(|e| RetryFailure::from(classify_reqwest(&e)))?;
                let status = resp.status();
                if status.is_success() {
                    return Ok(());
                }
                let hint = crate::errclass::retry_after_ms(&resp);
                let text = resp.text().await.unwrap_or_default();
                Err(RetryFailure {
                    error: classify_status(status, &text),
                    retry_after_ms: hint,
                })
            },
        )
        .await
    }

    async fn move_temp_to_final(&self) -> Result<(), BackendError> {
        let overwrite = if self.exclusive { "F" } else { "T" };
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .http
                    .request(move_method(), &self.temp_url)
                    .basic_auth(&self.username, Some(&self.password))
                    .header("Destination", self.final_url.as_str())
                    .header("Overwrite", overwrite)
                    .send()
                    .await
                    .map_err(|e| RetryFailure::from(classify_reqwest(&e)))?;
                let status = resp.status();
                if status.is_success() {
                    return Ok(());
                }
                let hint = crate::errclass::retry_after_ms(&resp);
                let text = resp.text().await.unwrap_or_default();
                Err(RetryFailure {
                    error: classify_status(status, &text),
                    retry_after_ms: hint,
                })
            },
        )
        .await
    }
}

impl Drop for WebDavWriter {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.spool_path);
    }
}

fn move_method() -> reqwest::Method {
    reqwest::Method::from_bytes(b"MOVE").expect("MOVE is a valid HTTP token")
}

#[async_trait]
impl StreamWriter for WebDavWriter {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        self.ensure_spool_file().await?;
        self.spool_file
            .as_mut()
            .expect("ensure_spool_file has just set spool_file")
            .write_all(data)
            .await?;
        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<(), BackendError> {
        if let Some(mut file) = self.spool_file.take() {
            file.flush().await?;
            drop(file);
        }

        if let Err(e) = self.put_temp_object().await {
            self.cleanup_local_spool().await;
            self.cleanup_remote_temp().await;
            return Err(e);
        }
        self.cleanup_local_spool().await;

        let result = self.move_temp_to_final().await;
        if result.is_err() {
            self.cleanup_remote_temp().await;
        }
        result
    }
}
