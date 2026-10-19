// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use cu_backend::resume::ResumeStore;
use cu_backend::{BackendError, CircuitBreaker, RetryPolicy, StreamWriter};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::backend::{DriveFile, escape_query_literal, parse_drive_file, trash_by_id};
use crate::errclass::{classify_reqwest, classify_status};
use crate::token::TokenSource;

pub const CHUNK_SIZE: usize = 8 * 1024 * 1024;

const ALIGN: usize = 256 * 1024;

const MAX_FINAL_ROUNDS: u32 = 8;

pub(crate) fn fresh_source_marker() -> [u8; 32] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&nanos.to_le_bytes());
    out[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    out
}

#[derive(Clone)]
pub(crate) struct DriveHandles {
    pub(crate) http: reqwest::Client,
    pub(crate) tokens: Arc<TokenSource>,
    pub(crate) files_url: String,
    pub(crate) upload_url: String,
    pub(crate) cache: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) retry_policy: RetryPolicy,
    pub(crate) breaker: Arc<CircuitBreaker>,
}

pub struct GDriveWriter {
    h: DriveHandles,
    remote_id: String,
    object_key: String,
    parent_id: String,
    name: String,
    existing_id: Option<String>,
    resume: Arc<dyn ResumeStore>,

    buf: Vec<u8>,
    buf_start: u64,
    bytes_seen: u64,
    resume_confirmed: u64,
    session_uri: Option<String>,
    already_completed_id: Option<String>,
    resume_checked: bool,
    exclusive: bool,
}

enum PutOutcome {
    Incomplete(u64),
    Completed(String),
}

impl GDriveWriter {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        h: DriveHandles,
        remote_id: String,
        object_key: String,
        parent_id: String,
        name: String,
        existing_id: Option<String>,
        resume: Arc<dyn ResumeStore>,
        exclusive: bool,
    ) -> Self {
        Self {
            h,
            remote_id,
            object_key,
            parent_id,
            name,
            existing_id,
            resume,
            buf: Vec::new(),
            buf_start: 0,
            bytes_seen: 0,
            resume_confirmed: 0,
            session_uri: None,
            already_completed_id: None,
            resume_checked: false,
            exclusive,
        }
    }

    async fn reconcile_exclusive(&self, new_id: &str) -> Result<(), BackendError> {
        let q = format!(
            "'{}' in parents and name = '{}' and trashed = false",
            escape_query_literal(&self.parent_id),
            escape_query_literal(&self.name)
        );
        let url = format!(
            "{}?q={}&fields=files(id,name,mimeType,createdTime)",
            self.h.files_url,
            cu_backend::percent::encode_segment(&q)
        );
        let token = self.h.tokens.get().await?;
        let resp = self
            .h
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(classify_status(status, &text));
        }
        let body: Value = serde_json::from_str(&text)
            .map_err(|e| BackendError::Other(format!("gdrive: invalid JSON response: {e}")))?;
        let mut matches: Vec<DriveFile> = body
            .get("files")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(parse_drive_file)
            .collect();
        if matches.is_empty() {
            return Ok(());
        }
        matches.sort_by(|a, b| a.created_time.cmp(&b.created_time));
        let canonical = matches.remove(0);
        for extra in &matches {
            let _ = trash_by_id(&self.h, &self.h.breaker, &extra.id).await;
        }
        if canonical.id == new_id {
            Ok(())
        } else {
            Err(BackendError::AlreadyExists)
        }
    }

    async fn check_resume(&mut self) -> Result<(), BackendError> {
        self.resume_checked = true;
        let Some(record) = self.resume.load(&self.remote_id, &self.object_key).await? else {
            return Ok(());
        };
        let Some(uri) = record.upload_id else {
            return Ok(());
        };
        match query_resume_offset(&self.h.http, &uri).await? {
            ResumeOffset::Bytes(n) => {
                self.session_uri = Some(uri);
                self.resume_confirmed = n;
            }
            ResumeOffset::Completed(id) => self.already_completed_id = Some(id),
            ResumeOffset::Expired => {}
        }
        Ok(())
    }

    async fn abandon_resume(&mut self) -> BackendError {
        self.session_uri = None;
        if let Err(e) = self.resume.abort(&self.remote_id, &self.object_key).await {
            return e;
        }
        BackendError::Transient(
            "gdrive: the upload resume point falls in the middle of a block, starting over"
                .to_string(),
        )
    }

    async fn ensure_session(&mut self) -> Result<String, BackendError> {
        if let Some(uri) = &self.session_uri {
            return Ok(uri.clone());
        }
        let uri = self.create_session().await?;
        self.resume
            .set_upload_id(&self.remote_id, &self.object_key, &uri)
            .await?;
        self.session_uri = Some(uri.clone());
        Ok(uri)
    }

    async fn create_session(&self) -> Result<String, BackendError> {
        let token = self.h.tokens.get().await?;
        let resp = self
            .h
            .http
            .post(&self.h.upload_url)
            .query(&[("uploadType", "resumable")])
            .bearer_auth(&token)
            .json(&json!({"name": self.name, "parents": [self.parent_id]}))
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, &text));
        }
        resp.headers()
            .get("Location")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .ok_or_else(|| {
                BackendError::Other("gdrive: resumable session without Location header".to_string())
            })
    }

    async fn put_range(
        &self,
        uri: &str,
        content_range: String,
        body: Bytes,
    ) -> Result<PutOutcome, BackendError> {
        let resp = self
            .h
            .http
            .put(uri)
            .header("Content-Range", content_range)
            .body(body)
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        match status.as_u16() {
            200 | 201 => {
                let text = resp.text().await.unwrap_or_default();
                let body: Value = serde_json::from_str(&text).map_err(|e| {
                    BackendError::Other(format!("gdrive: invalid JSON response: {e}"))
                })?;
                body.get("id")
                    .and_then(Value::as_str)
                    .map(|id| PutOutcome::Completed(id.to_string()))
                    .ok_or_else(|| {
                        BackendError::Other("gdrive: upload completed without id".to_string())
                    })
            }
            308 => Ok(PutOutcome::Incomplete(confirmed_from_range(
                resp.headers().get("Range").and_then(|v| v.to_str().ok()),
            ))),
            _ => {
                let text = resp.text().await.unwrap_or_default();
                Err(classify_status(status, &text))
            }
        }
    }

    fn absorb(&mut self, piece: &Bytes, confirmed: u64) -> Result<bool, BackendError> {
        let end = self.buf_start + piece.len() as u64;
        if confirmed < self.buf_start || confirmed > end {
            return Err(BackendError::Other(format!(
                "gdrive: Drive reports {confirmed} bytes received, expected between {} and {end}",
                self.buf_start
            )));
        }
        let progressed = usize::try_from(confirmed - self.buf_start).unwrap_or(usize::MAX);
        if progressed < piece.len() {
            let mut restored = piece[progressed..].to_vec();
            restored.extend_from_slice(&self.buf);
            self.buf = restored;
        }
        self.buf_start = confirmed;
        Ok(progressed > 0)
    }

    async fn flush_aligned(&mut self) -> Result<(), BackendError> {
        let len = self.buf.len() / ALIGN * ALIGN;
        if len == 0 {
            return Ok(());
        }
        let uri = self.ensure_session().await?;
        let content_range = format!(
            "bytes {}-{}/*",
            self.buf_start,
            self.buf_start + len as u64 - 1
        );
        let rest = self.buf.split_off(len);
        let piece = Bytes::from(std::mem::replace(&mut self.buf, rest));
        match self.put_range(&uri, content_range, piece.clone()).await? {
            PutOutcome::Incomplete(confirmed) => {
                if !self.absorb(&piece, confirmed)? {
                    return Err(BackendError::Transient(
                        "gdrive: Drive confirmed no byte of the chunk".to_string(),
                    ));
                }
                Ok(())
            }
            PutOutcome::Completed(_) => Err(BackendError::Other(
                "gdrive: session completed by a non-final chunk".to_string(),
            )),
        }
    }

    async fn flush_final(&mut self) -> Result<String, BackendError> {
        if let Some(id) = self.already_completed_id.clone() {
            return Ok(id);
        }
        if self.bytes_seen < self.resume_confirmed {
            return Err(self.abandon_resume().await);
        }
        let uri = self.ensure_session().await?;
        let total = self.bytes_seen;
        for _ in 0..MAX_FINAL_ROUNDS {
            let content_range = if self.buf.is_empty() {
                format!("bytes */{total}")
            } else {
                format!("bytes {}-{}/{total}", self.buf_start, total - 1)
            };
            let piece = Bytes::from(std::mem::take(&mut self.buf));
            match self.put_range(&uri, content_range, piece.clone()).await? {
                PutOutcome::Completed(id) => return Ok(id),
                PutOutcome::Incomplete(confirmed) => {
                    if !self.absorb(&piece, confirmed)? {
                        return Err(BackendError::Transient(
                            "gdrive: Drive confirmed no byte of the final chunk".to_string(),
                        ));
                    }
                }
            }
        }
        Err(BackendError::Transient(
            "gdrive: final chunk only partly confirmed too many times".to_string(),
        ))
    }

    async fn trash(&self, id: &str, breaker: &CircuitBreaker) -> Result<(), BackendError> {
        trash_by_id(&self.h, breaker, id).await
    }
}

fn confirmed_from_range(range: Option<&str>) -> u64 {
    range
        .and_then(|r| r.strip_prefix("bytes=0-"))
        .and_then(|end| end.parse::<u64>().ok())
        .map_or(0, |end| end + 1)
}

enum ResumeOffset {
    Bytes(u64),
    Completed(String),
    Expired,
}

async fn query_resume_offset(
    http: &reqwest::Client,
    uri: &str,
) -> Result<ResumeOffset, BackendError> {
    let resp = http
        .put(uri)
        .header("Content-Range", "bytes */*")
        .header("Content-Length", "0")
        .send()
        .await
        .map_err(|e| classify_reqwest(&e))?;
    match resp.status().as_u16() {
        200 | 201 => {
            let text = resp.text().await.unwrap_or_default();
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let id = body
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_default();
            Ok(ResumeOffset::Completed(id))
        }
        308 => Ok(ResumeOffset::Bytes(confirmed_from_range(
            resp.headers().get("Range").and_then(|v| v.to_str().ok()),
        ))),
        404 | 410 => Ok(ResumeOffset::Expired),
        other => {
            let text = resp.text().await.unwrap_or_default();
            Err(classify_status(
                reqwest::StatusCode::from_u16(other).unwrap_or(reqwest::StatusCode::BAD_GATEWAY),
                &text,
            ))
        }
    }
}

#[async_trait]
impl StreamWriter for GDriveWriter {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        if !self.resume_checked {
            self.check_resume().await?;
        }
        let start = self.bytes_seen;
        let end = start + data.len() as u64;
        self.bytes_seen = end;
        if self.already_completed_id.is_some() || end <= self.resume_confirmed {
            self.buf_start = end;
            return Ok(());
        }
        if start < self.resume_confirmed {
            return Err(self.abandon_resume().await);
        }
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK_SIZE {
            self.flush_aligned().await?;
        }
        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<(), BackendError> {
        if !self.resume_checked {
            self.check_resume().await?;
        }
        let new_id = self.flush_final().await?;
        self.resume
            .finish(&self.remote_id, &self.object_key)
            .await?;
        if self.exclusive {
            debug_assert!(
                self.existing_id.is_none(),
                "an exclusive write never has an existing_id to replace"
            );
            let outcome = self.reconcile_exclusive(&new_id).await;
            if outcome.is_ok() {
                self.h
                    .cache
                    .lock()
                    .await
                    .insert(self.object_key.clone(), new_id);
            }
            return outcome;
        }
        if let Some(old_id) = self.existing_id.clone() {
            let breaker = Arc::clone(&self.h.breaker);
            if let Err(e) = self.trash(&old_id, &breaker).await {
                let cleanup_breaker = CircuitBreaker::s3_default();
                let _ = self.trash(&new_id, &cleanup_breaker).await;
                self.h.cache.lock().await.remove(&self.object_key);
                return Err(e);
            }
        }
        self.h
            .cache
            .lock()
            .await
            .insert(self.object_key.clone(), new_id);
        Ok(())
    }
}
