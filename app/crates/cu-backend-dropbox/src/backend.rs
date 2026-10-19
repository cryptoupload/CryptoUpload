// SPDX-License-Identifier: GPL-3.0-or-later

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cu_backend::jitter_seed;
use cu_backend::resume::ResumeStore;
use cu_backend::{
    BackendError, Capabilities, CircuitBreaker, Clock, EntryKind, ListEntry, ObjPath, ObjStat,
    RetryFailure, RetryPolicy, StorageBackend, StreamReader, StreamWriter, SystemClock,
    retry_with_hint,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::config::DropboxConfig;
use crate::errclass::{classify_reqwest, classify_status, retry_after_ms};
use crate::reader::DropboxReader;
use crate::writer::DropboxWriter;

const DEFAULT_API_ROOT: &str = "https://api.dropboxapi.com";
const DEFAULT_CONTENT_ROOT: &str = "https://content.dropboxapi.com";

pub(crate) type SleepFn =
    Arc<dyn Fn(u64) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

fn real_sleep(ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(tokio::time::sleep(Duration::from_millis(ms)))
}

fn http_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120));
    if let Some(proxy) = cu_backend::proxy::global() {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy.url()).expect("proxy address already validated when read"),
        );
    }
    builder
        .build()
        .expect("HTTP client build: only TLS backend initialization can fail here")
}

fn dropbox_path(path: &ObjPath) -> String {
    if path.is_root() {
        String::new()
    } else {
        format!("/{}", path.as_str())
    }
}

pub struct DropboxBackend {
    pub(crate) http: reqwest::Client,
    pub(crate) config: DropboxConfig,
    pub(crate) remote_id: String,
    pub(crate) resume: Arc<dyn ResumeStore>,
    access_token: Mutex<Option<(String, Instant)>>,
    api_root: String,
    content_root: String,
    retry_policy: RetryPolicy,
    breaker: Arc<CircuitBreaker>,
    clock: Arc<dyn Clock>,
    sleep: SleepFn,
}

impl DropboxBackend {
    #[must_use]
    pub fn new(config: &DropboxConfig, remote_id: String, resume: Arc<dyn ResumeStore>) -> Self {
        Self {
            http: http_client(),
            config: config.clone(),
            remote_id,
            resume,
            access_token: Mutex::new(None),
            api_root: DEFAULT_API_ROOT.to_string(),
            content_root: DEFAULT_CONTENT_ROOT.to_string(),
            retry_policy: RetryPolicy::s3_default(),
            breaker: Arc::new(CircuitBreaker::s3_default()),
            clock: Arc::new(SystemClock),
            sleep: Arc::new(real_sleep),
        }
    }

    pub(crate) async fn access_token(&self) -> Result<String, BackendError> {
        {
            let guard = self.access_token.lock().await;
            if let Some((token, expires_at)) = guard.as_ref()
                && *expires_at > Instant::now() + Duration::from_secs(30)
            {
                return Ok(token.clone());
            }
        }
        self.force_refresh_access_token().await
    }

    async fn force_refresh_access_token(&self) -> Result<String, BackendError> {
        let (token, expires_in) = crate::oauth::refresh_access_token_with_expiry(
            &self.config.client_id,
            &self.config.refresh_token,
        )
        .await?;
        let expires_at = Instant::now() + expires_in.unwrap_or(Duration::from_secs(1800));
        *self.access_token.lock().await = Some((token.clone(), expires_at));
        Ok(token)
    }

    async fn send_authed<F>(&self, mut build: F) -> Result<reqwest::Response, BackendError>
    where
        F: FnMut(&str) -> reqwest::RequestBuilder,
    {
        let token = self.access_token().await?;
        let resp = build(&token)
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        if resp.status().as_u16() != 401 {
            return Ok(resp);
        }
        let token = self.force_refresh_access_token().await?;
        build(&token).send().await.map_err(|e| classify_reqwest(&e))
    }

    async fn call_rpc(&self, path: &str, body: &Value) -> Result<Value, BackendError> {
        let url = format!("{}{path}", self.api_root);
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .send_authed(|token| self.http.post(&url).bearer_auth(token).json(body))
                    .await
                    .map_err(RetryFailure::from)?;
                let status = resp.status();
                let hint = retry_after_ms(&resp);
                let text = resp.text().await.unwrap_or_default();
                if !status.is_success() {
                    return Err(RetryFailure {
                        error: classify_status(status, &text),
                        retry_after_ms: hint,
                    });
                }
                serde_json::from_str::<Value>(&text).map_err(|e| {
                    RetryFailure::from(BackendError::Other(format!(
                        "dropbox: invalid JSON response: {e}"
                    )))
                })
            },
        )
        .await
    }

    async fn get_metadata(&self, path: &ObjPath) -> Result<Value, BackendError> {
        self.call_rpc(
            "/2/files/get_metadata",
            &json!({"path": dropbox_path(path)}),
        )
        .await
    }
}

#[async_trait]
impl StorageBackend for DropboxBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            atomic_write: true,
            native_rename: true,
            conditional_create: true,
            random_read: false,
            stale_path_cache: false,
        }
    }

    async fn exists(&self, path: &ObjPath) -> Result<bool, BackendError> {
        if path.is_root() {
            return Ok(true);
        }
        match self.get_metadata(path).await {
            Ok(_) => Ok(true),
            Err(BackendError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn get(&self, path: &ObjPath) -> Result<Vec<u8>, BackendError> {
        let mut reader = self.open_read(path).await?;
        let mut out = Vec::new();
        loop {
            let chunk = reader.read_chunk(1024 * 1024).await?;
            if chunk.is_empty() {
                break;
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    async fn get_range(
        &self,
        path: &ObjPath,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let url = format!("{}/2/files/download", self.content_root);
        let arg = json!({"path": dropbox_path(path)}).to_string();
        let range = format!(
            "bytes={offset}-{}",
            offset.saturating_add(len).saturating_sub(1)
        );
        let resp = retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .send_authed(|token| {
                        self.http
                            .post(&url)
                            .bearer_auth(token)
                            .header("Dropbox-API-Arg", arg.clone())
                            .header(reqwest::header::RANGE, &range)
                    })
                    .await
                    .map_err(RetryFailure::from)?;
                let status = resp.status();
                if !status.is_success() {
                    let hint = retry_after_ms(&resp);
                    let text = resp.text().await.unwrap_or_default();
                    return Err(RetryFailure {
                        error: classify_status(status, &text),
                        retry_after_ms: hint,
                    });
                }
                Ok(resp)
            },
        )
        .await?;

        let got_range = resp.status().as_u16() == 206;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| classify_reqwest(&e))?
            .to_vec();
        if got_range {
            return Ok(bytes);
        }
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        let want = usize::try_from(len).unwrap_or(usize::MAX);
        let end = start.saturating_add(want).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }

    async fn stat(&self, path: &ObjPath) -> Result<ObjStat, BackendError> {
        let meta = self.get_metadata(path).await?;
        let size = meta.get("size").and_then(Value::as_u64).unwrap_or(0);
        Ok(ObjStat { size })
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        let mut writer = self.open_write(path).await?;
        writer.write_chunk(data).await?;
        writer.commit().await
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        let url = format!("{}/2/files/download", self.content_root);
        let arg = json!({"path": dropbox_path(path)}).to_string();
        let resp = retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .send_authed(|token| {
                        self.http
                            .post(&url)
                            .bearer_auth(token)
                            .header("Dropbox-API-Arg", arg.clone())
                    })
                    .await
                    .map_err(RetryFailure::from)?;
                let status = resp.status();
                if !status.is_success() {
                    let hint = retry_after_ms(&resp);
                    let text = resp.text().await.unwrap_or_default();
                    return Err(RetryFailure {
                        error: classify_status(status, &text),
                        retry_after_ms: hint,
                    });
                }
                Ok(resp)
            },
        )
        .await?;
        Ok(Box::new(DropboxReader::new(resp)))
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        Ok(Box::new(DropboxWriter::new(
            self.http.clone(),
            self.access_token().await?,
            self.content_root.clone(),
            self.remote_id.clone(),
            path.as_str().to_string(),
            dropbox_path(path),
            Arc::clone(&self.resume),
            false,
        )))
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        Ok(Box::new(DropboxWriter::new(
            self.http.clone(),
            self.access_token().await?,
            self.content_root.clone(),
            self.remote_id.clone(),
            path.as_str().to_string(),
            dropbox_path(path),
            Arc::clone(&self.resume),
            true,
        )))
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        self.delete_path(path).await
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        let mut entries = Vec::new();
        let mut body = self
            .call_rpc("/2/files/list_folder", &json!({"path": dropbox_path(dir)}))
            .await?;
        loop {
            let items = body
                .get("entries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for item in &items {
                if let Some(entry) = parse_list_entry(item) {
                    entries.push(entry);
                }
            }
            let has_more = body
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !has_more {
                break;
            }
            let cursor = body
                .get("cursor")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    BackendError::Other("dropbox: list_folder has_more without cursor".to_string())
                })?;
            body = self
                .call_rpc("/2/files/list_folder/continue", &json!({"cursor": cursor}))
                .await?;
        }
        Ok(entries)
    }

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        if dir.is_root() {
            return Ok(());
        }
        let mut current = ObjPath::root();
        for seg in dir.as_str().split('/') {
            current = current.join(seg)?;
            match self
                .call_rpc(
                    "/2/files/create_folder_v2",
                    &json!({"path": dropbox_path(&current)}),
                )
                .await
            {
                Ok(_) | Err(BackendError::AlreadyExists) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        _from_kind: EntryKind,
    ) -> Result<(), BackendError> {
        self.call_rpc(
            "/2/files/move_v2",
            &json!({"from_path": dropbox_path(from), "to_path": dropbox_path(to)}),
        )
        .await?;
        Ok(())
    }

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        self.delete_path(dir).await
    }

    async fn load_resume_prologue(
        &self,
        path: &ObjPath,
        content_hash: Option<[u8; 32]>,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        let key = path.as_str();
        let Some(record) = self.resume.load(&self.remote_id, key).await? else {
            return Ok(None);
        };
        if record.source_hash != content_hash {
            self.resume.abort(&self.remote_id, key).await?;
            return Ok(None);
        }
        Ok(record.prologue)
    }

    async fn store_resume_prologue(
        &self,
        path: &ObjPath,
        bytes: &[u8],
        content_hash: Option<[u8; 32]>,
    ) -> Result<(), BackendError> {
        let key = path.as_str();
        let source_hash = content_hash.unwrap_or_else(crate::writer::fresh_source_marker);
        self.resume
            .begin(
                &self.remote_id,
                key,
                crate::writer::CHUNK_SIZE as u64,
                source_hash,
            )
            .await?;
        self.resume.set_prologue(&self.remote_id, key, bytes).await
    }
}

impl DropboxBackend {
    async fn delete_path(&self, path: &ObjPath) -> Result<(), BackendError> {
        match self
            .call_rpc("/2/files/delete_v2", &json!({"path": dropbox_path(path)}))
            .await
        {
            Ok(_) | Err(BackendError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn parse_list_entry(v: &Value) -> Option<ListEntry> {
    let tag = v.get(".tag").and_then(Value::as_str)?;
    let name = v.get("name").and_then(Value::as_str)?.to_string();
    match tag {
        "file" => Some(ListEntry {
            name,
            kind: EntryKind::File,
            size: v.get("size").and_then(Value::as_u64),
            modified: v
                .get("server_modified")
                .and_then(Value::as_str)
                .and_then(cu_backend::timefmt::parse_rfc3339),
        }),
        "folder" => Some(ListEntry {
            name,
            kind: EntryKind::Directory,
            size: None,
            modified: None,
        }),
        _ => None,
    }
}
