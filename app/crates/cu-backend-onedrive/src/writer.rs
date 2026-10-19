// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cu_backend::resume::ResumeStore;
use cu_backend::{
    BackendError, CircuitBreaker, Clock, RetryFailure, RetryPolicy, StreamWriter, jitter_seed,
    retry_with_hint,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::backend::SleepFn;
use crate::errclass::{classify_reqwest, classify_status, retry_after_ms};
use crate::token::TokenSource;

pub const CHUNK_SIZE: usize = 10 * 320 * 1024;

pub const SMALL_PUT_THRESHOLD: usize = 4 * 1024 * 1024;

static SPOOL_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn fresh_source_marker() -> [u8; 32] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&nanos.to_le_bytes());
    out[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    out
}

fn fresh_spool_path() -> PathBuf {
    static SWEEP: std::sync::Once = std::sync::Once::new();
    SWEEP.call_once(|| {
        let _ = cu_backend::spool::sweep_stale(
            &std::env::temp_dir(),
            "cu-onedrive-",
            ".part",
            cu_backend::spool::STALE_SPOOL_AGE,
        );
    });
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let counter = SPOOL_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cu-onedrive-{nanos:x}-{}-{counter:x}.part",
        std::process::id()
    ))
}

#[derive(Clone)]
pub(crate) struct WriterHandles {
    pub(crate) http: reqwest::Client,
    pub(crate) tokens: Arc<TokenSource>,
    pub(crate) graph_root: String,
    pub(crate) retry_policy: RetryPolicy,
    pub(crate) breaker: Arc<CircuitBreaker>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) sleep: SleepFn,
}

enum FragmentOutcome {
    Next(u64),
    Done,
}

pub struct OneDriveWriter {
    h: WriterHandles,
    item_addr: String,
    remote_id: String,
    object_key: String,
    resume: Arc<dyn ResumeStore>,

    buf: Vec<u8>,
    spool_path: PathBuf,
    spool_file: Option<tokio::fs::File>,
    total: u64,
    boundaries: Vec<u64>,
    exclusive: bool,
}

impl OneDriveWriter {
    pub(crate) fn new(
        h: WriterHandles,
        item_addr: String,
        remote_id: String,
        object_key: String,
        resume: Arc<dyn ResumeStore>,
        exclusive: bool,
    ) -> Self {
        Self {
            h,
            item_addr,
            remote_id,
            object_key,
            resume,
            buf: Vec::new(),
            spool_path: fresh_spool_path(),
            spool_file: None,
            total: 0,
            boundaries: Vec::new(),
            exclusive,
        }
    }

    async fn with_retry<T, F, Fut>(&self, mut attempt: F) -> Result<T, BackendError>
    where
        F: FnMut(u32) -> Fut,
        Fut: std::future::Future<Output = Result<T, RetryFailure>>,
    {
        retry_with_hint(
            &self.h.retry_policy,
            &self.h.breaker,
            self.h.clock.as_ref(),
            jitter_seed(),
            |ms| (self.h.sleep)(ms),
            &mut attempt,
        )
        .await
    }

    async fn failure(&self, resp: reqwest::Response) -> RetryFailure {
        let status = resp.status();
        if status.as_u16() == 401 {
            return match self.h.tokens.refresh().await {
                Ok(_) => BackendError::Transient("onedrive: 401, token renewed".to_string()),
                Err(e) => e,
            }
            .into();
        }
        let hint = retry_after_ms(&resp);
        let text = resp.text().await.unwrap_or_default();
        RetryFailure {
            error: classify_status(status, &text),
            retry_after_ms: hint,
        }
    }

    async fn single_upload(&self, data: bytes::Bytes) -> Result<(), BackendError> {
        let url = format!("{}{}:/content", self.h.graph_root, self.item_addr);
        self.with_retry(|_| async {
            let token = self.h.tokens.get().await?;
            let resp = self
                .h
                .http
                .put(&url)
                .bearer_auth(&token)
                .body(data.clone())
                .send()
                .await
                .map_err(|e| classify_reqwest(&e))?;
            if resp.status().is_success() {
                return Ok(());
            }
            Err(self.failure(resp).await)
        })
        .await
    }

    async fn create_session(&self, conflict_behavior: &str) -> Result<String, BackendError> {
        let url = format!(
            "{}{}:/createUploadSession",
            self.h.graph_root, self.item_addr
        );
        let body = json!({"item": {"@microsoft.graph.conflictBehavior": conflict_behavior}});
        let text = self
            .with_retry(|_| async {
                let token = self.h.tokens.get().await?;
                let resp = self
                    .h
                    .http
                    .post(&url)
                    .bearer_auth(&token)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| classify_reqwest(&e))?;
                if resp.status().is_success() {
                    return Ok(resp.text().await.unwrap_or_default());
                }
                Err(self.failure(resp).await)
            })
            .await?;
        let body: Value = serde_json::from_str(&text)
            .map_err(|e| BackendError::Other(format!("onedrive: invalid JSON response: {e}")))?;
        body.get("uploadUrl")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                BackendError::Other("onedrive: createUploadSession without uploadUrl".to_string())
            })
    }

    async fn query_next_expected(&self, upload_url: &str) -> Option<u64> {
        let resp = self.h.http.get(upload_url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let text = resp.text().await.ok()?;
        let body: Value = serde_json::from_str(&text).ok()?;
        let ranges = body.get("nextExpectedRanges").and_then(Value::as_array)?;
        match ranges.first().and_then(Value::as_str) {
            Some(range) => range.split('-').next()?.parse::<u64>().ok(),
            None => Some(u64::MAX),
        }
    }

    async fn ensure_session(
        &mut self,
        conflict_behavior: &str,
    ) -> Result<(String, u64), BackendError> {
        if let Some(record) = self.resume.load(&self.remote_id, &self.object_key).await?
            && let Some(url) = record.upload_id
            && let Some(start) = self.query_next_expected(&url).await
        {
            let resumable =
                start == 0 || start == u64::MAX || self.boundaries.binary_search(&start).is_ok();
            if !resumable || (start != u64::MAX && start > self.total) {
                self.resume.abort(&self.remote_id, &self.object_key).await?;
                return Err(BackendError::Transient(
                    "onedrive: the upload resume point falls inside a block, \
                             restarting from zero"
                        .to_string(),
                ));
            }
            return Ok((url, start));
        }
        let url = self.create_session(conflict_behavior).await?;
        self.resume
            .set_upload_id(&self.remote_id, &self.object_key, &url)
            .await?;
        Ok((url, 0))
    }

    async fn read_range(&mut self, start: u64, end: u64) -> Result<Vec<u8>, BackendError> {
        let len = usize::try_from(end - start).unwrap_or(usize::MAX);
        if self.spool_file.is_none() {
            let s = usize::try_from(start).unwrap_or(usize::MAX);
            return Ok(self.buf[s..s + len].to_vec());
        }
        let mut file = tokio::fs::File::open(&self.spool_path).await?;
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut out = vec![0u8; len];
        file.read_exact(&mut out).await?;
        Ok(out)
    }

    async fn send_fragment(
        &self,
        upload_url: &str,
        fragment: &[u8],
        offset: u64,
    ) -> Result<FragmentOutcome, BackendError> {
        let end = offset + fragment.len() as u64;
        let total = self.total;
        self.with_retry(|attempt| async move {
            let mut from = offset;
            if attempt > 0 {
                match self.query_next_expected(upload_url).await {
                    Some(u64::MAX) => return Ok(FragmentOutcome::Done),
                    Some(n) if n >= end => return Ok(FragmentOutcome::Next(n)),
                    Some(n) if n > offset => from = n,
                    _ => {}
                }
            }
            let skip = usize::try_from(from - offset).unwrap_or(usize::MAX);
            let resp = self
                .h
                .http
                .put(upload_url)
                .header(
                    reqwest::header::CONTENT_RANGE,
                    format!("bytes {from}-{}/{total}", end - 1),
                )
                .body(fragment[skip..].to_vec())
                .send()
                .await
                .map_err(|e| classify_reqwest(&e))?;
            match resp.status().as_u16() {
                200 | 201 => Ok(FragmentOutcome::Done),
                202 => Ok(FragmentOutcome::Next(end)),
                _ => Err(self.failure(resp).await),
            }
        })
        .await
    }

    async fn upload_via_session(&mut self) -> Result<(), BackendError> {
        let conflict_behavior = if self.exclusive { "fail" } else { "replace" };
        let (upload_url, mut offset) = self.ensure_session(conflict_behavior).await?;
        while offset < self.total {
            let end = (offset + CHUNK_SIZE as u64).min(self.total);
            let fragment = self.read_range(offset, end).await?;
            match self.send_fragment(&upload_url, &fragment, offset).await? {
                FragmentOutcome::Done => return Ok(()),
                FragmentOutcome::Next(n) => offset = n,
            }
        }
        Ok(())
    }

    async fn cleanup_spool(&mut self) {
        if self.spool_file.take().is_some() {
            let _ = tokio::fs::remove_file(&self.spool_path).await;
        }
    }
}

#[async_trait]
impl StreamWriter for OneDriveWriter {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        self.total += data.len() as u64;
        self.boundaries.push(self.total);
        if self.spool_file.is_none() && self.buf.len() + data.len() < SMALL_PUT_THRESHOLD {
            self.buf.extend_from_slice(data);
            return Ok(());
        }
        if self.spool_file.is_none() {
            let mut options = tokio::fs::OpenOptions::new();
            options.create(true).write(true).truncate(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&self.spool_path).await?;
            file.write_all(&std::mem::take(&mut self.buf)).await?;
            self.spool_file = Some(file);
        }
        let file = self.spool_file.as_mut().expect("just opened");
        file.write_all(data).await?;
        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<(), BackendError> {
        if let Some(file) = self.spool_file.as_mut() {
            file.flush().await?;
        }
        let result = if !self.exclusive && self.spool_file.is_none() {
            let body = std::mem::take(&mut self.buf);
            self.single_upload(bytes::Bytes::from(body)).await
        } else {
            self.upload_via_session().await
        };
        self.cleanup_spool().await;
        result?;
        self.resume.finish(&self.remote_id, &self.object_key).await
    }
}

impl Drop for OneDriveWriter {
    fn drop(&mut self) {
        if self.spool_file.is_some() {
            let _ = std::fs::remove_file(&self.spool_path);
        }
    }
}
