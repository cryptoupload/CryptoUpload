// SPDX-License-Identifier: GPL-3.0-or-later

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cu_backend::jitter_seed;
use cu_backend::resume::ResumeStore;
use cu_backend::{
    BackendError, Capabilities, CircuitBreaker, Clock, EntryKind, ListEntry, ObjPath, ObjStat,
    RetryFailure, RetryPolicy, StorageBackend, StreamReader, StreamWriter, SystemClock,
    retry_with_hint,
};

use crate::config::WebDavConfig;
use crate::errclass::{classify_reqwest, classify_status, retry_after_ms};
use crate::reader::WebDavReader;
use crate::writer::WebDavWriter;
use crate::xml::{children_from_multistatus, stat_from_multistatus};

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

fn propfind_method() -> reqwest::Method {
    reqwest::Method::from_bytes(b"PROPFIND").expect("PROPFIND is a valid HTTP token")
}

fn mkcol_method() -> reqwest::Method {
    reqwest::Method::from_bytes(b"MKCOL").expect("MKCOL is a valid HTTP token")
}

fn move_method() -> reqwest::Method {
    reqwest::Method::from_bytes(b"MOVE").expect("MOVE is a valid HTTP token")
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><D:propfind xmlns:D="DAV:"><D:prop><D:resourcetype/><D:getcontentlength/><D:getlastmodified/></D:prop></D:propfind>"#;

pub struct WebDavBackend {
    http: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    root_prefix: String,
    remote_id: String,
    resume: Arc<dyn ResumeStore>,
    retry_policy: RetryPolicy,
    breaker: Arc<CircuitBreaker>,
    clock: Arc<dyn Clock>,
    sleep: SleepFn,
    spool_dir: std::path::PathBuf,
}

impl WebDavBackend {
    pub(crate) async fn list_including_temp(
        &self,
        dir: &ObjPath,
    ) -> Result<Vec<ListEntry>, BackendError> {
        let url = self.collection_url(dir);
        let body = self.propfind(&url, "1").await?;
        let self_path = Self::request_path(&url);
        children_from_multistatus(&body, &self_path)
    }

    pub fn new(
        config: &WebDavConfig,
        remote_id: String,
        resume: Arc<dyn ResumeStore>,
    ) -> Result<Self, BackendError> {
        config.validate()?;
        Ok(Self {
            http: http_client(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            username: config.username.clone(),
            password: config.password.clone(),
            root_prefix: config.root_path.trim_matches('/').to_string(),
            remote_id,
            resume,
            retry_policy: RetryPolicy::s3_default(),
            breaker: Arc::new(CircuitBreaker::s3_default()),
            clock: Arc::new(SystemClock),
            sleep: Arc::new(real_sleep),
            spool_dir: std::env::temp_dir(),
        })
    }

    fn logical_path(&self, path: &ObjPath) -> String {
        if path.is_root() {
            self.root_prefix.clone()
        } else if self.root_prefix.is_empty() {
            path.as_str().to_string()
        } else {
            format!("{}/{}", self.root_prefix, path.as_str())
        }
    }

    fn url_from_logical(&self, logical: &str) -> String {
        if logical.is_empty() {
            return format!("{}/", self.base_url);
        }
        let encoded = logical
            .split('/')
            .map(cu_backend::percent::encode_segment)
            .collect::<Vec<_>>()
            .join("/");
        format!("{}/{encoded}", self.base_url)
    }

    fn object_url(&self, path: &ObjPath) -> String {
        self.url_from_logical(&self.logical_path(path))
    }

    fn collection_url(&self, path: &ObjPath) -> String {
        let mut url = self.object_url(path);
        if !url.ends_with('/') {
            url.push('/');
        }
        url
    }

    fn request_path(url: &str) -> String {
        reqwest::Url::parse(url)
            .map(|u| cu_backend::percent::decode(u.path()))
            .unwrap_or_default()
    }

    async fn propfind(&self, url: &str, depth: &str) -> Result<String, BackendError> {
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .http
                    .request(propfind_method(), url)
                    .basic_auth(&self.username, Some(&self.password))
                    .header("Depth", depth)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/xml; charset=utf-8",
                    )
                    .body(PROPFIND_BODY)
                    .send()
                    .await
                    .map_err(|e| RetryFailure::from(classify_reqwest(&e)))?;
                let status = resp.status();
                let hint = retry_after_ms(&resp);
                let text = resp.text().await.unwrap_or_default();
                if !status.is_success() {
                    return Err(RetryFailure {
                        error: classify_status(status, &text),
                        retry_after_ms: hint,
                    });
                }
                Ok(text)
            },
        )
        .await
    }

    async fn delete_url(&self, url: &str) -> Result<(), BackendError> {
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .http
                    .delete(url)
                    .basic_auth(&self.username, Some(&self.password))
                    .send()
                    .await
                    .map_err(|e| RetryFailure::from(classify_reqwest(&e)))?;
                let status = resp.status();
                if status.is_success() || status.as_u16() == 404 {
                    return Ok(());
                }
                let hint = retry_after_ms(&resp);
                let text = resp.text().await.unwrap_or_default();
                Err(RetryFailure {
                    error: classify_status(status, &text),
                    retry_after_ms: hint,
                })
            },
        )
        .await
    }

    async fn mkcol(&self, url: &str) -> Result<(), BackendError> {
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .http
                    .request(mkcol_method(), url)
                    .basic_auth(&self.username, Some(&self.password))
                    .send()
                    .await
                    .map_err(|e| RetryFailure::from(classify_reqwest(&e)))?;
                let status = resp.status();
                if status.is_success() || status.as_u16() == 405 {
                    return Ok(());
                }
                let hint = retry_after_ms(&resp);
                let text = resp.text().await.unwrap_or_default();
                Err(RetryFailure {
                    error: classify_status(status, &text),
                    retry_after_ms: hint,
                })
            },
        )
        .await
    }

    async fn create_dir_all_logical(&self, logical: &str) -> Result<(), BackendError> {
        if logical.is_empty() {
            return Ok(());
        }
        let mut current = String::new();
        for seg in logical.split('/') {
            current = if current.is_empty() {
                seg.to_string()
            } else {
                format!("{current}/{seg}")
            };
            let url = self.url_from_logical(&current);
            let url = if url.ends_with('/') {
                url
            } else {
                format!("{url}/")
            };
            self.mkcol(&url).await?;
        }
        Ok(())
    }

    async fn ensure_parent_collection(&self, path: &ObjPath) -> Result<(), BackendError> {
        let parent = parent_path(path)?;
        self.create_dir_all_logical(&self.logical_path(&parent))
            .await
    }
}

fn parent_path(path: &ObjPath) -> Result<ObjPath, BackendError> {
    match path.as_str().rfind('/') {
        Some(i) => ObjPath::new(path.as_str()[..i].to_string()),
        None => Ok(ObjPath::root()),
    }
}

#[async_trait]
impl StorageBackend for WebDavBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            atomic_write: true,
            native_rename: true,
            conditional_create: true,
            random_read: true,
            stale_path_cache: false,
        }
    }

    async fn exists(&self, path: &ObjPath) -> Result<bool, BackendError> {
        if path.is_root() && self.root_prefix.is_empty() {
            return Ok(true);
        }
        match self.stat(path).await {
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
        let url = self.object_url(path);
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
                    .http
                    .get(&url)
                    .basic_auth(&self.username, Some(&self.password))
                    .header(reqwest::header::RANGE, &range)
                    .send()
                    .await
                    .map_err(|e| RetryFailure::from(classify_reqwest(&e)))?;
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
        let url = self.object_url(path);
        let body = self.propfind(&url, "0").await?;
        let (_, size) = stat_from_multistatus(&body)?;
        Ok(ObjStat {
            size: size.unwrap_or(0),
        })
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        let mut writer = self.open_write(path).await?;
        writer.write_chunk(data).await?;
        writer.commit().await
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        let url = self.object_url(path);
        let resp = retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .http
                    .get(&url)
                    .basic_auth(&self.username, Some(&self.password))
                    .send()
                    .await
                    .map_err(|e| RetryFailure::from(classify_reqwest(&e)))?;
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
        Ok(Box::new(WebDavReader::new(resp)))
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        self.ensure_parent_collection(path).await?;
        Ok(Box::new(WebDavWriter::new(
            self.http.clone(),
            self.object_url(path),
            self.username.clone(),
            self.password.clone(),
            false,
            self.retry_policy,
            Arc::clone(&self.breaker),
            Arc::clone(&self.clock),
            Arc::clone(&self.sleep),
            &self.spool_dir,
        )))
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        self.ensure_parent_collection(path).await?;
        Ok(Box::new(WebDavWriter::new(
            self.http.clone(),
            self.object_url(path),
            self.username.clone(),
            self.password.clone(),
            true,
            self.retry_policy,
            Arc::clone(&self.breaker),
            Arc::clone(&self.clock),
            Arc::clone(&self.sleep),
            &self.spool_dir,
        )))
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        self.delete_url(&self.object_url(path)).await
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        let mut entries = self.list_including_temp(dir).await?;
        entries.retain(|e| !e.name.starts_with(crate::writer::TEMP_PREFIX));
        Ok(entries)
    }

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        self.create_dir_all_logical(&self.logical_path(dir)).await
    }

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        from_kind: EntryKind,
    ) -> Result<(), BackendError> {
        self.ensure_parent_collection(to).await?;
        let (from_url, to_url) = match from_kind {
            EntryKind::File => (self.object_url(from), self.object_url(to)),
            EntryKind::Directory => (self.collection_url(from), self.collection_url(to)),
        };
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .http
                    .request(move_method(), &from_url)
                    .basic_auth(&self.username, Some(&self.password))
                    .header("Destination", to_url.as_str())
                    .header("Overwrite", "F")
                    .send()
                    .await
                    .map_err(|e| RetryFailure::from(classify_reqwest(&e)))?;
                let status = resp.status();
                if status.is_success() {
                    return Ok(());
                }
                let hint = retry_after_ms(&resp);
                let text = resp.text().await.unwrap_or_default();
                Err(RetryFailure {
                    error: classify_status(status, &text),
                    retry_after_ms: hint,
                })
            },
        )
        .await
    }

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        self.delete_url(&self.collection_url(dir)).await
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
            .begin(&self.remote_id, key, 0, source_hash)
            .await?;
        self.resume.set_prologue(&self.remote_id, key, bytes).await
    }
}
