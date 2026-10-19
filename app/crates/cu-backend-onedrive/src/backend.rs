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
use reqwest::Method;
use serde_json::{Value, json};

use crate::config::OneDriveConfig;
use crate::errclass::{classify_reqwest, classify_status, retry_after_ms};
use crate::reader::OneDriveReader;
use crate::token::{RefreshTokenSink, TokenSource};
use crate::writer::{OneDriveWriter, WriterHandles};

const DEFAULT_GRAPH_ROOT: &str = "https://graph.microsoft.com/v1.0";

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

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(cu_backend::percent::encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

pub struct OneDriveBackend {
    pub(crate) http: reqwest::Client,
    pub(crate) config: OneDriveConfig,
    pub(crate) remote_id: String,
    pub(crate) resume: Arc<dyn ResumeStore>,
    tokens: Arc<TokenSource>,
    graph_root: String,
    retry_policy: RetryPolicy,
    breaker: Arc<CircuitBreaker>,
    clock: Arc<dyn Clock>,
    sleep: SleepFn,
}

impl OneDriveBackend {
    #[must_use]
    pub fn new(config: &OneDriveConfig, remote_id: String, resume: Arc<dyn ResumeStore>) -> Self {
        Self {
            http: http_client(),
            config: config.clone(),
            remote_id,
            resume,
            tokens: Arc::new(TokenSource::new(config)),
            graph_root: DEFAULT_GRAPH_ROOT.to_string(),
            retry_policy: RetryPolicy::s3_default(),
            breaker: Arc::new(CircuitBreaker::s3_default()),
            clock: Arc::new(SystemClock),
            sleep: Arc::new(real_sleep),
        }
    }

    pub fn set_refresh_token_sink(&self, sink: RefreshTokenSink) {
        self.tokens.set_sink(sink);
    }

    pub(crate) fn writer_handles(&self) -> WriterHandles {
        WriterHandles {
            http: self.http.clone(),
            tokens: Arc::clone(&self.tokens),
            graph_root: self.graph_root.clone(),
            retry_policy: self.retry_policy,
            breaker: Arc::clone(&self.breaker),
            clock: Arc::clone(&self.clock),
            sleep: Arc::clone(&self.sleep),
        }
    }

    pub(crate) async fn access_token(&self) -> Result<String, BackendError> {
        self.tokens.get().await
    }

    async fn force_refresh_access_token(&self) -> Result<String, BackendError> {
        self.tokens.refresh().await
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

    async fn call_graph(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Value, BackendError> {
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            self.clock.as_ref(),
            jitter_seed(),
            |ms| (self.sleep)(ms),
            |_attempt| async {
                let resp = self
                    .send_authed(|token| {
                        let mut req = self.http.request(method.clone(), url).bearer_auth(token);
                        if let Some(b) = body {
                            req = req.json(b);
                        }
                        req
                    })
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
                if text.is_empty() {
                    return Ok(Value::Null);
                }
                serde_json::from_str::<Value>(&text).map_err(|e| {
                    RetryFailure::from(BackendError::Other(format!(
                        "onedrive: invalid JSON response: {e}"
                    )))
                })
            },
        )
        .await
    }

    fn root_addr(&self) -> &'static str {
        self.config.root_kind.root_addr()
    }

    pub(crate) fn item_addr(&self, path: &ObjPath) -> String {
        if path.is_root() {
            self.root_addr().to_string()
        } else {
            format!("{}:/{}", self.root_addr(), encode_path(path.as_str()))
        }
    }

    fn children_addr(&self, dir: &ObjPath) -> String {
        if dir.is_root() {
            format!("{}/children", self.root_addr())
        } else {
            format!(
                "{}:/{}:/children",
                self.root_addr(),
                encode_path(dir.as_str())
            )
        }
    }

    fn parent_reference_path(&self, parent: &ObjPath) -> String {
        let root_no_me = self.root_addr().trim_start_matches("/me");
        if parent.is_root() {
            format!("{root_no_me}:")
        } else {
            format!("{root_no_me}:/{}", encode_path(parent.as_str()))
        }
    }

    async fn get_metadata(
        &self,
        path: &ObjPath,
        select: Option<&str>,
    ) -> Result<Value, BackendError> {
        let mut url = format!("{}{}", self.graph_root, self.item_addr(path));
        if let Some(select) = select {
            url = format!("{url}?$select={select}");
        }
        self.call_graph(Method::GET, &url, None).await
    }

    async fn delete_path(&self, path: &ObjPath) -> Result<(), BackendError> {
        let url = format!("{}{}", self.graph_root, self.item_addr(path));
        match self.call_graph(Method::DELETE, &url, None).await {
            Ok(_) | Err(BackendError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

pub(crate) fn same_origin(base: &str, candidate: &str) -> bool {
    let (Ok(base), Ok(candidate)) = (reqwest::Url::parse(base), reqwest::Url::parse(candidate))
    else {
        return false;
    };
    candidate.username().is_empty()
        && candidate.password().is_none()
        && base.scheme() == candidate.scheme()
        && base.host_str().map(str::to_ascii_lowercase)
            == candidate.host_str().map(str::to_ascii_lowercase)
        && base.port_or_known_default() == candidate.port_or_known_default()
}

#[async_trait]
impl StorageBackend for OneDriveBackend {
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
        if path.is_root() {
            return Ok(true);
        }
        match self.get_metadata(path, Some("id,size,file,folder")).await {
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
        let meta = self.get_metadata(path, None).await?;
        let download_url = meta
            .get("@microsoft.graph.downloadUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                BackendError::Other("onedrive: response without downloadUrl".to_string())
            })?
            .to_string();
        let range = format!(
            "bytes={offset}-{}",
            offset.saturating_add(len).saturating_sub(1)
        );
        let resp = self
            .http
            .get(&download_url)
            .header(reqwest::header::RANGE, &range)
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, &text));
        }
        Ok(resp
            .bytes()
            .await
            .map_err(|e| classify_reqwest(&e))?
            .to_vec())
    }

    async fn stat(&self, path: &ObjPath) -> Result<ObjStat, BackendError> {
        let meta = self.get_metadata(path, Some("id,size,file,folder")).await?;
        let size = meta.get("size").and_then(Value::as_u64).unwrap_or(0);
        Ok(ObjStat { size })
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        let mut writer = self.open_write(path).await?;
        writer.write_chunk(data).await?;
        writer.commit().await
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        let meta = self.get_metadata(path, None).await?;
        let download_url = meta
            .get("@microsoft.graph.downloadUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                BackendError::Other("onedrive: response without downloadUrl".to_string())
            })?
            .to_string();
        let resp = self
            .http
            .get(&download_url)
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, &text));
        }
        Ok(Box::new(OneDriveReader::new(resp)))
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        Ok(Box::new(OneDriveWriter::new(
            self.writer_handles(),
            self.item_addr(path),
            self.remote_id.clone(),
            path.as_str().to_string(),
            Arc::clone(&self.resume),
            false,
        )))
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        Ok(Box::new(OneDriveWriter::new(
            self.writer_handles(),
            self.item_addr(path),
            self.remote_id.clone(),
            path.as_str().to_string(),
            Arc::clone(&self.resume),
            true,
        )))
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        self.delete_path(path).await
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        let mut entries = Vec::new();
        let mut url = Some(format!(
            "{}{}?$select=name,size,file,folder&$top=200",
            self.graph_root,
            self.children_addr(dir)
        ));
        while let Some(u) = url {
            let body = self.call_graph(Method::GET, &u, None).await?;
            for item in body
                .get("value")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(entry) = parse_list_entry(item) {
                    entries.push(entry);
                }
            }
            url = match body.get("@odata.nextLink").and_then(Value::as_str) {
                Some(next) if same_origin(&self.graph_root, next) => Some(next.to_string()),
                Some(_) => return Err(crate::errclass::foreign_next_link()),
                None => None,
            };
        }
        Ok(entries)
    }

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        if dir.is_root() {
            return Ok(());
        }
        let mut current = ObjPath::root();
        for seg in dir.as_str().split('/') {
            let parent = current.clone();
            current = current.join(seg)?;
            let url = format!("{}{}", self.graph_root, self.children_addr(&parent));
            match self
                .call_graph(
                    Method::POST,
                    &url,
                    Some(&json!({
                        "name": seg,
                        "folder": {},
                        "@microsoft.graph.conflictBehavior": "fail",
                    })),
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
        let (parent, name) = split_parent_name(to);
        let url = format!("{}{}", self.graph_root, self.item_addr(from));
        self.call_graph(
            Method::PATCH,
            &url,
            Some(&json!({
                "name": name,
                "parentReference": {"path": self.parent_reference_path(&parent)},
            })),
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

fn split_parent_name(path: &ObjPath) -> (ObjPath, String) {
    match path.as_str().rsplit_once('/') {
        Some((parent, name)) => (
            ObjPath::new(parent).unwrap_or_else(|_| ObjPath::root()),
            name.to_string(),
        ),
        None => (ObjPath::root(), path.as_str().to_string()),
    }
}

fn parse_list_entry(v: &Value) -> Option<ListEntry> {
    let name = v.get("name").and_then(Value::as_str)?.to_string();
    let kind = if v.get("folder").is_some() {
        EntryKind::Directory
    } else if v.get("file").is_some() {
        EntryKind::File
    } else {
        return None;
    };
    Some(ListEntry {
        name,
        kind,
        size: v.get("size").and_then(Value::as_u64),
        modified: v
            .get("lastModifiedDateTime")
            .and_then(Value::as_str)
            .and_then(cu_backend::timefmt::parse_rfc3339),
    })
}
