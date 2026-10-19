// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cu_backend::resume::ResumeStore;
use cu_backend::{BackendError, StreamWriter};
use serde_json::{Value, json};

use crate::errclass::{classify_reqwest, classify_status};

pub const CHUNK_SIZE: usize = 8 * 1024 * 1024;

const MAX_OFFSET_CORRECTIONS: u32 = 4;

pub(crate) fn fresh_source_marker() -> [u8; 32] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&nanos.to_le_bytes());
    out[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    out
}

enum AppendOutcome {
    Ok,
    IncorrectOffset(u64),
}

pub struct DropboxWriter {
    http: reqwest::Client,
    token: String,
    content_root: String,
    remote_id: String,
    object_key: String,
    dropbox_path: String,
    resume: Arc<dyn ResumeStore>,

    buf: Vec<u8>,
    session_id: Option<String>,
    confirmed: u64,
    position: u64,
    resume_checked: bool,
    exclusive: bool,
}

impl DropboxWriter {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        http: reqwest::Client,
        token: String,
        content_root: String,
        remote_id: String,
        object_key: String,
        dropbox_path: String,
        resume: Arc<dyn ResumeStore>,
        exclusive: bool,
    ) -> Self {
        Self {
            http,
            token,
            content_root,
            remote_id,
            object_key,
            dropbox_path,
            resume,
            buf: Vec::new(),
            session_id: None,
            confirmed: 0,
            position: 0,
            resume_checked: false,
            exclusive,
        }
    }

    fn mode_tag(&self) -> Value {
        if self.exclusive {
            json!({".tag": "add"})
        } else {
            json!({".tag": "overwrite"})
        }
    }

    fn commit_info(&self) -> Value {
        json!({
            "path": self.dropbox_path,
            "mode": self.mode_tag(),
            "autorename": false,
        })
    }

    async fn single_upload(&self, data: &[u8]) -> Result<String, BackendError> {
        let url = format!("{}/2/files/upload", self.content_root);
        let arg = self.commit_info().to_string();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .header("Dropbox-API-Arg", arg)
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(classify_status(status, &text));
        }
        Ok(self.dropbox_path.clone())
    }

    async fn start_session(&self) -> Result<String, BackendError> {
        let url = format!("{}/2/files/upload_session/start", self.content_root);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .header("Dropbox-API-Arg", json!({"close": false}).to_string())
            .body(Vec::new())
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(classify_status(status, &text));
        }
        let body: Value = serde_json::from_str(&text)
            .map_err(|e| BackendError::Other(format!("dropbox: invalid JSON response: {e}")))?;
        body.get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                BackendError::Other("dropbox: upload_session/start without session_id".to_string())
            })
    }

    async fn ensure_session(&mut self) -> Result<(), BackendError> {
        if self.session_id.is_some() {
            return Ok(());
        }
        if !self.resume_checked {
            self.resume_checked = true;
            if let Some(record) = self.resume.load(&self.remote_id, &self.object_key).await?
                && let Some(session_id) = record.upload_id
            {
                self.session_id = Some(session_id);
                self.confirmed = 0;
                return Ok(());
            }
        }
        let session_id = self.start_session().await?;
        self.resume
            .set_upload_id(&self.remote_id, &self.object_key, &session_id)
            .await?;
        self.session_id = Some(session_id);
        Ok(())
    }

    async fn try_append(
        &self,
        session_id: &str,
        offset: u64,
        data: &[u8],
    ) -> Result<AppendOutcome, BackendError> {
        let url = format!("{}/2/files/upload_session/append_v2", self.content_root);
        let arg = json!({
            "cursor": {"session_id": session_id, "offset": offset},
            "close": false,
        })
        .to_string();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .header("Dropbox-API-Arg", arg)
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(AppendOutcome::Ok);
        }
        if status.as_u16() == 409
            && let Some(correct_offset) = parse_incorrect_offset(&text)
        {
            return Ok(AppendOutcome::IncorrectOffset(correct_offset));
        }
        Err(classify_status(status, &text))
    }

    async fn append_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        let session_id = self
            .session_id
            .clone()
            .expect("ensure_session has just set session_id");
        let start = self.position;
        let end = start + data.len() as u64;
        self.position = end;
        if self.confirmed >= end {
            return Ok(());
        }
        let mut offset = self.confirmed.max(start);
        for _ in 0..MAX_OFFSET_CORRECTIONS {
            let skip = usize::try_from(offset - start).unwrap_or(usize::MAX);
            match self.try_append(&session_id, offset, &data[skip..]).await? {
                AppendOutcome::Ok => {
                    self.confirmed = end;
                    return Ok(());
                }
                AppendOutcome::IncorrectOffset(correct_offset) => {
                    if correct_offset >= end {
                        self.confirmed = correct_offset;
                        return Ok(());
                    }
                    if correct_offset < start {
                        return Err(self.abandon_session().await);
                    }
                    offset = correct_offset;
                }
            }
        }
        Err(BackendError::Other(
            "dropbox: too many consecutive offset corrections in the upload session".to_string(),
        ))
    }

    async fn abandon_session(&self) -> BackendError {
        if let Err(e) = self.resume.abort(&self.remote_id, &self.object_key).await {
            return e;
        }
        BackendError::Transient(
            "dropbox: the resumed upload session does not match the file, starting over"
                .to_string(),
        )
    }

    async fn finish_session(&mut self, data: &[u8]) -> Result<String, BackendError> {
        let session_id = self
            .session_id
            .clone()
            .expect("ensure_session has just set session_id");
        let url = format!("{}/2/files/upload_session/finish", self.content_root);
        let start = self.position;
        let end = start + data.len() as u64;
        if self.confirmed > end {
            return Err(self.abandon_session().await);
        }
        let mut offset = self.confirmed.max(start);
        for _ in 0..MAX_OFFSET_CORRECTIONS {
            let skip = usize::try_from(offset - start).unwrap_or(usize::MAX);
            let arg = json!({
                "cursor": {"session_id": session_id, "offset": offset},
                "commit": self.commit_info(),
            })
            .to_string();
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&self.token)
                .header("Dropbox-API-Arg", arg)
                .body(data[skip..].to_vec())
                .send()
                .await
                .map_err(|e| classify_reqwest(&e))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if status.is_success() {
                self.position = end;
                self.confirmed = end;
                return Ok(self.dropbox_path.clone());
            }
            match (status.as_u16(), parse_incorrect_offset(&text)) {
                (409, Some(correct)) if correct >= start && correct <= end => offset = correct,
                (409, Some(_)) => return Err(self.abandon_session().await),
                _ => return Err(classify_status(status, &text)),
            }
        }
        Err(BackendError::Other(
            "dropbox: too many consecutive offset corrections in the upload session".to_string(),
        ))
    }

    async fn flush(&mut self, final_chunk: bool) -> Result<Option<String>, BackendError> {
        if final_chunk && self.session_id.is_none() {
            let data = std::mem::take(&mut self.buf);
            return self.single_upload(&data).await.map(Some);
        }
        self.ensure_session().await?;
        let data = std::mem::take(&mut self.buf);
        if final_chunk {
            Ok(Some(self.finish_session(&data).await?))
        } else {
            self.append_chunk(&data).await?;
            Ok(None)
        }
    }
}

fn parse_incorrect_offset(body: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(body).ok()?;
    let mut error = value.get("error")?;
    if error.get(".tag").and_then(Value::as_str) == Some("lookup_failed") {
        error = error.get("lookup_failed")?;
    }
    if error.get(".tag").and_then(Value::as_str) != Some("incorrect_offset") {
        return None;
    }
    error.get("correct_offset").and_then(Value::as_u64)
}

#[async_trait]
impl StreamWriter for DropboxWriter {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= CHUNK_SIZE {
            self.flush(false).await?;
        }
        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<(), BackendError> {
        let _path = self
            .flush(true)
            .await?
            .expect("flush(final_chunk=true) always returns a path");
        self.resume.finish(&self.remote_id, &self.object_key).await
    }
}
