// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use cu_backend::jitter_seed;
use cu_backend::resume::ResumeStore;
use cu_backend::{
    BackendError, Capabilities, CircuitBreaker, EntryKind, ListEntry, ObjPath, ObjStat,
    RetryFailure, RetryPolicy, StorageBackend, StreamReader, StreamWriter, SystemClock,
    emulate_rename_via_copy_and_delete, retry_with_hint,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::config::GDriveConfig;
use crate::errclass::{classify_reqwest, classify_status, retry_after_ms};
use crate::reader::GDriveReader;
use crate::token::TokenSource;
use crate::writer::{DriveHandles, GDriveWriter};

const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(15);

pub(crate) type IdCache = Arc<Mutex<HashMap<String, String>>>;

const MAX_LIST_PAGES: usize = 100_000;

const DEFAULT_API_ROOT: &str = "https://www.googleapis.com";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

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

#[derive(Debug, Clone)]
pub(crate) struct DriveFile {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) created_time: String,
    pub(crate) size: Option<u64>,
    pub(crate) modified: Option<i64>,
}

pub(crate) async fn sleep_ms(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

pub(crate) async fn trash_by_id(
    handles: &DriveHandles,
    breaker: &CircuitBreaker,
    id: &str,
) -> Result<(), BackendError> {
    let url = format!("{}/{id}", handles.files_url);
    retry_with_hint(
        &handles.retry_policy,
        breaker,
        &SystemClock,
        jitter_seed(),
        sleep_ms,
        |_attempt| async {
            let token = handles.tokens.get().await?;
            let resp = handles
                .http
                .patch(&url)
                .bearer_auth(&token)
                .json(&json!({"trashed": true}))
                .send()
                .await
                .map_err(|e| classify_reqwest(&e))?;
            let status = resp.status();
            if status.is_success() || status.as_u16() == 404 {
                return Ok(());
            }
            if status.as_u16() == 401 {
                handles.tokens.refresh().await?;
                return Err(BackendError::Transient(
                    "gdrive: 401 while trashing, token renewed".to_string(),
                )
                .into());
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

pub(crate) fn parse_drive_file(v: &Value) -> Option<DriveFile> {
    Some(DriveFile {
        id: v.get("id")?.as_str()?.to_string(),
        name: v.get("name")?.as_str()?.to_string(),
        is_dir: v.get("mimeType").and_then(Value::as_str) == Some(FOLDER_MIME),
        created_time: v
            .get("createdTime")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        size: v
            .get("size")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok()),
        modified: v
            .get("modifiedTime")
            .and_then(Value::as_str)
            .and_then(cu_backend::timefmt::parse_rfc3339),
    })
}

pub struct GDriveBackend {
    pub(crate) http: reqwest::Client,
    pub(crate) config: GDriveConfig,
    pub(crate) remote_id: String,
    pub(crate) resume: Arc<dyn ResumeStore>,
    cache: IdCache,
    cache_born: Arc<std::sync::Mutex<std::time::Instant>>,
    cache_ttl: std::time::Duration,
    tokens: Arc<TokenSource>,
    api_root: String,
    retry_policy: RetryPolicy,
    breaker: Arc<CircuitBreaker>,
}

impl GDriveBackend {
    #[must_use]
    pub fn new(config: &GDriveConfig, remote_id: String, resume: Arc<dyn ResumeStore>) -> Self {
        Self::with_parts(
            config.clone(),
            remote_id,
            resume,
            TokenSource::new(config),
            DEFAULT_API_ROOT.to_string(),
        )
    }

    fn with_parts(
        config: GDriveConfig,
        remote_id: String,
        resume: Arc<dyn ResumeStore>,
        tokens: TokenSource,
        api_root: String,
    ) -> Self {
        Self {
            http: http_client(),
            config,
            remote_id,
            resume,
            cache: Arc::new(Mutex::new(HashMap::new())),
            cache_born: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
            cache_ttl: CACHE_TTL,
            tokens: Arc::new(tokens),
            api_root,
            retry_policy: RetryPolicy::s3_default(),
            breaker: Arc::new(CircuitBreaker::s3_default()),
        }
    }

    fn files_url(&self) -> String {
        format!("{}/drive/v3/files", self.api_root)
    }

    fn upload_url(&self) -> String {
        format!("{}/upload/drive/v3/files", self.api_root)
    }

    pub(crate) fn handles(&self) -> DriveHandles {
        DriveHandles {
            http: self.http.clone(),
            tokens: Arc::clone(&self.tokens),
            files_url: self.files_url(),
            upload_url: self.upload_url(),
            cache: Arc::clone(&self.cache),
            retry_policy: self.retry_policy,
            breaker: Arc::clone(&self.breaker),
        }
    }

    pub fn set_refresh_token_sink(&self, sink: crate::token::RefreshTokenSink) {
        self.tokens.set_sink(sink);
    }

    pub(crate) async fn access_token(&self) -> Result<String, BackendError> {
        self.tokens.get().await
    }

    async fn invalidate_prefix(&self, path: &ObjPath) {
        let mut cache = self.cache.lock().await;
        if path.is_root() {
            cache.clear();
            return;
        }
        let prefix = format!("{}/", path.as_str());
        cache.retain(|k, _| k != path.as_str() && !k.starts_with(&prefix));
    }

    async fn cache_get(&self, key: &str) -> Option<String> {
        let mut cache = self.cache.lock().await;
        let expired = {
            let mut born = self.cache_born.lock().unwrap();
            if born.elapsed() >= self.cache_ttl {
                *born = std::time::Instant::now();
                true
            } else {
                false
            }
        };
        if expired {
            cache.clear();
            return None;
        }
        cache.get(key).cloned()
    }

    async fn get_json(&self, url: &str) -> Result<Value, BackendError> {
        let token = self.access_token().await?;
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                let resp = self
                    .http
                    .get(url)
                    .bearer_auth(&token)
                    .send()
                    .await
                    .map_err(|e| classify_reqwest(&e))?;
                let status = resp.status();
                let hint = retry_after_ms(&resp);
                let text = resp.text().await.unwrap_or_default();
                if !status.is_success() {
                    return Err(RetryFailure {
                        error: classify_status(status, &text),
                        retry_after_ms: hint,
                    });
                }
                serde_json::from_str(&text).map_err(|e| {
                    BackendError::Other(format!("gdrive: invalid JSON response: {e}")).into()
                })
            },
        )
        .await
    }

    async fn list_files(&self, q: &str) -> Result<Vec<DriveFile>, BackendError> {
        let base = format!(
            "{}?q={}&fields=nextPageToken,files(id,name,mimeType,createdTime,modifiedTime,size)&pageSize=1000",
            self.files_url(),
            cu_backend::percent::encode_segment(q)
        );
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let url = match &page_token {
                Some(t) => format!(
                    "{base}&pageToken={}",
                    cu_backend::percent::encode_segment(t)
                ),
                None => base.clone(),
            };
            let body = self.get_json(&url).await?;
            if let Some(files) = body.get("files").and_then(Value::as_array) {
                out.extend(files.iter().filter_map(parse_drive_file));
            }
            match body.get("nextPageToken").and_then(Value::as_str) {
                None => return Ok(out),
                Some(t) if Some(t) == page_token.as_deref() => {
                    return Err(BackendError::Other(
                        "gdrive: files.list repeated the same nextPageToken".to_string(),
                    ));
                }
                Some(t) => page_token = Some(t.to_string()),
            }
        }
        Err(BackendError::Other(
            "gdrive: files.list never stops returning pages".to_string(),
        ))
    }

    async fn list_children(&self, parent_id: &str) -> Result<Vec<DriveFile>, BackendError> {
        self.list_files(&format!(
            "'{}' in parents and trashed = false",
            escape_query_literal(parent_id)
        ))
        .await
    }

    async fn trash(&self, id: &str) -> Result<(), BackendError> {
        trash_by_id(&self.handles(), &self.breaker, id).await
    }

    async fn dedup(&self, mut matches: Vec<DriveFile>) -> Result<Option<DriveFile>, BackendError> {
        if matches.len() <= 1 {
            return Ok(matches.pop());
        }
        matches.sort_by(|a, b| a.created_time.cmp(&b.created_time));
        let canonical = matches.remove(0);
        for extra in &matches {
            self.trash(&extra.id).await?;
        }
        Ok(Some(canonical))
    }

    async fn find_named(
        &self,
        parent_id: &str,
        name: &str,
    ) -> Result<Option<DriveFile>, BackendError> {
        let matches = self
            .list_files(&format!(
                "'{}' in parents and name = '{}' and trashed = false",
                escape_query_literal(parent_id),
                escape_query_literal(name)
            ))
            .await?;
        self.dedup(matches).await
    }

    async fn create_folder(&self, parent_id: &str, name: &str) -> Result<String, BackendError> {
        let token = self.access_token().await?;
        let url = self.files_url();
        let body = retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |attempt| {
                let (url, token) = (&url, &token);
                async move {
                    if attempt > 0
                        && let Some(existing) =
                            self.find_named(parent_id, name).await?.filter(|f| f.is_dir)
                    {
                        return Ok(json!({"id": existing.id}));
                    }
                    let resp = self
                        .http
                        .post(url)
                        .bearer_auth(token)
                        .json(&json!({
                            "name": name,
                            "mimeType": FOLDER_MIME,
                            "parents": [parent_id],
                        }))
                        .send()
                        .await
                        .map_err(|e| classify_reqwest(&e))?;
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
                        BackendError::Other(format!("gdrive: invalid JSON response: {e}")).into()
                    })
                }
            },
        )
        .await?;
        body.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| BackendError::Other("gdrive: files.create without id".to_string()))
    }

    pub async fn find_or_create_root_folder(&self, name: &str) -> Result<String, BackendError> {
        self.find_or_create_folder("root", name).await
    }

    async fn find_or_create_folder(
        &self,
        parent_id: &str,
        name: &str,
    ) -> Result<String, BackendError> {
        if let Some(existing) = self.find_named(parent_id, name).await? {
            return Ok(existing.id);
        }
        self.create_folder(parent_id, name).await
    }

    async fn resolve_dir_id(
        &self,
        dir: &ObjPath,
        create_missing: bool,
    ) -> Result<String, BackendError> {
        if dir.is_root() {
            return Ok(self.config.folder_id.clone());
        }
        if let Some(cached) = self.cache_get(dir.as_str()).await {
            return Ok(cached);
        }
        let mut current = self.config.folder_id.clone();
        for seg in dir.as_str().split('/') {
            current = if create_missing {
                self.find_or_create_folder(&current, seg).await?
            } else {
                self.find_named(&current, seg)
                    .await?
                    .filter(|f| f.is_dir)
                    .map(|f| f.id)
                    .ok_or(BackendError::NotFound)?
            };
        }
        self.cache
            .lock()
            .await
            .insert(dir.as_str().to_string(), current.clone());
        Ok(current)
    }

    pub(crate) async fn resolve_id(&self, path: &ObjPath) -> Result<Option<String>, BackendError> {
        if path.is_root() {
            return Ok(Some(self.config.folder_id.clone()));
        }
        if let Some(cached) = self.cache_get(path.as_str()).await {
            return Ok(Some(cached));
        }
        let (parent, name) = split_parent_name(path.as_str());
        let parent_id = match self.resolve_dir_id(&parent, false).await {
            Ok(id) => id,
            Err(BackendError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let found = self.find_named(&parent_id, &name).await?;
        if let Some(file) = &found {
            self.cache
                .lock()
                .await
                .insert(path.as_str().to_string(), file.id.clone());
        }
        Ok(found.map(|f| f.id))
    }

    pub(crate) async fn resolve_parent_for_write(
        &self,
        path: &ObjPath,
    ) -> Result<(String, String), BackendError> {
        let (parent, name) = split_parent_name(path.as_str());
        let parent_id = self.resolve_dir_id(&parent, true).await?;
        Ok((parent_id, name))
    }
}

fn split_parent_name(path: &str) -> (ObjPath, String) {
    match path.rsplit_once('/') {
        Some((parent, name)) => (
            ObjPath::new(parent).unwrap_or_else(|_| ObjPath::root()),
            name.to_string(),
        ),
        None => (ObjPath::root(), path.to_string()),
    }
}

pub(crate) fn escape_query_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

#[async_trait]
impl StorageBackend for GDriveBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            atomic_write: true,
            native_rename: false,
            conditional_create: true,
            random_read: true,
            stale_path_cache: true,
        }
    }

    async fn exists(&self, path: &ObjPath) -> Result<bool, BackendError> {
        Ok(self.resolve_id(path).await?.is_some())
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
        let id = self.resolve_id(path).await?.ok_or(BackendError::NotFound)?;
        let url = format!("{}/{id}?alt=media", self.files_url());
        let range = format!(
            "bytes={offset}-{}",
            offset.saturating_add(len).saturating_sub(1)
        );
        let token = self.access_token().await?;
        retry_with_hint(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                let resp = self
                    .http
                    .get(&url)
                    .bearer_auth(&token)
                    .header(reqwest::header::RANGE, &range)
                    .send()
                    .await
                    .map_err(|e| classify_reqwest(&e))?;
                let status = resp.status();
                if status.as_u16() == 416 {
                    return Ok(Vec::new());
                }
                if !status.is_success() {
                    let hint = retry_after_ms(&resp);
                    let text = resp.text().await.unwrap_or_default();
                    return Err(RetryFailure {
                        error: classify_status(status, &text),
                        retry_after_ms: hint,
                    });
                }
                resp.bytes()
                    .await
                    .map(|b| b.to_vec())
                    .map_err(|e| classify_reqwest(&e).into())
            },
        )
        .await
    }

    async fn stat(&self, path: &ObjPath) -> Result<ObjStat, BackendError> {
        let id = self.resolve_id(path).await?.ok_or(BackendError::NotFound)?;
        let url = format!("{}/{id}?fields=size", self.files_url());
        let body = self.get_json(&url).await?;
        let size = body
            .get("size")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        Ok(ObjStat { size })
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        let mut writer = self.open_write(path).await?;
        writer.write_chunk(data).await?;
        writer.commit().await
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        let id = self.resolve_id(path).await?.ok_or(BackendError::NotFound)?;
        Ok(Box::new(GDriveReader::new(
            self.http.clone(),
            self.access_token().await?,
            self.files_url(),
            id,
        )))
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        let (parent_id, name) = self.resolve_parent_for_write(path).await?;
        let existing_id = self.find_named(&parent_id, &name).await?.map(|f| f.id);
        Ok(Box::new(GDriveWriter::new(
            self.handles(),
            self.remote_id.clone(),
            path.as_str().to_string(),
            parent_id,
            name,
            existing_id,
            Arc::clone(&self.resume),
            false,
        )))
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        let (parent_id, name) = self.resolve_parent_for_write(path).await?;
        Ok(Box::new(GDriveWriter::new(
            self.handles(),
            self.remote_id.clone(),
            path.as_str().to_string(),
            parent_id,
            name,
            None,
            Arc::clone(&self.resume),
            true,
        )))
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        if let Some(id) = self.resolve_id(path).await? {
            self.trash(&id).await?;
        }
        self.invalidate_prefix(path).await;
        Ok(())
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        let id = self.resolve_dir_id(dir, false).await?;
        let children = self.list_children(&id).await?;
        let mut by_name: HashMap<String, Vec<DriveFile>> = HashMap::new();
        for child in children {
            by_name.entry(child.name.clone()).or_default().push(child);
        }
        let mut entries = Vec::with_capacity(by_name.len());
        for (name, matches) in by_name {
            if let Some(file) = self.dedup(matches).await? {
                entries.push(ListEntry {
                    name,
                    kind: if file.is_dir {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    size: file.size,
                    modified: file.modified,
                });
            }
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        self.resolve_dir_id(dir, true).await?;
        Ok(())
    }

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        from_kind: EntryKind,
    ) -> Result<(), BackendError> {
        emulate_rename_via_copy_and_delete(self, from, to, from_kind).await?;
        if from_kind == EntryKind::Directory
            && let Some(id) = self.resolve_id(from).await?
        {
            self.trash(&id).await?;
        }
        self.invalidate_prefix(from).await;
        self.invalidate_prefix(to).await;
        Ok(())
    }

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        if dir.is_root() {
            let root_id = self.config.folder_id.clone();
            for child in self.list_children(&root_id).await? {
                self.trash(&child.id).await?;
            }
        } else if let Some(id) = self.resolve_id(dir).await? {
            self.trash(&id).await?;
        }
        self.invalidate_prefix(dir).await;
        Ok(())
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
