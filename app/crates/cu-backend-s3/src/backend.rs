// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use cu_backend::jitter_seed;
use cu_backend::resume::ResumeStore;
use cu_backend::{
    BackendError, Capabilities, CircuitBreaker, EntryKind, ListEntry, ObjPath, ObjStat,
    RetryPolicy, StorageBackend, StreamReader, StreamWriter, SystemClock,
    emulate_remove_dir_all_via_delete, retry,
};

use crate::config::{ObjectLockMode, S3Config, retain_until_date};
use crate::errclass::classify;
use crate::objkey::{list_prefix, normalize_prefix, object_key};
use crate::reader::S3Reader;
use crate::writer::S3Writer;

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

pub struct S3Backend {
    pub(crate) client: Client,
    pub(crate) bucket: String,
    pub(crate) prefix: String,
    pub(crate) remote_id: String,
    pub(crate) resume: Arc<dyn ResumeStore>,
    pub(crate) retry_policy: RetryPolicy,
    pub(crate) breaker: Arc<CircuitBreaker>,
    pub(crate) object_lock_mode: Option<ObjectLockMode>,
    pub(crate) object_lock_days: Option<u16>,
}

impl S3Backend {
    #[must_use]
    pub fn new(cfg: &S3Config, remote_id: String, resume: Arc<dyn ResumeStore>) -> Self {
        Self {
            client: cfg.client(),
            bucket: cfg.bucket.clone(),
            prefix: normalize_prefix(&cfg.prefix),
            remote_id,
            resume,
            retry_policy: RetryPolicy::s3_default(),
            breaker: Arc::new(CircuitBreaker::s3_default()),
            object_lock_mode: cfg.object_lock_mode,
            object_lock_days: cfg.object_lock_days,
        }
    }

    pub async fn check_object_lock_enabled(cfg: &S3Config) -> Result<(), BackendError> {
        let client = cfg.client();
        let not_enabled = || {
            BackendError::Coded(cu_backend::Msg::new("S3_OBJECT_LOCK_NOT_ENABLED").arg(&cfg.bucket))
        };
        match client
            .get_object_lock_configuration()
            .bucket(&cfg.bucket)
            .send()
            .await
        {
            Ok(out) => {
                let enabled = out.object_lock_configuration().is_some_and(|c| {
                    matches!(
                        c.object_lock_enabled(),
                        Some(aws_sdk_s3::types::ObjectLockEnabled::Enabled)
                    )
                });
                if enabled { Ok(()) } else { Err(not_enabled()) }
            }
            Err(e) if e.code() == Some("ObjectLockConfigurationNotFoundError") => {
                Err(not_enabled())
            }
            Err(e) => Err(classify(&e)),
        }
    }

    fn key(&self, path: &ObjPath) -> String {
        object_key(&self.prefix, path)
    }

    async fn head(&self, key: &str) -> Result<Option<i64>, BackendError> {
        let result = retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                self.client
                    .head_object()
                    .bucket(&self.bucket)
                    .key(key)
                    .send()
                    .await
                    .map_err(|e| {
                        if e.as_service_error().is_some_and(
                            aws_sdk_s3::operation::head_object::HeadObjectError::is_not_found,
                        ) {
                            BackendError::NotFound
                        } else {
                            classify(&e)
                        }
                    })
            },
        )
        .await;
        match result {
            Ok(out) => Ok(Some(out.content_length().unwrap_or(0))),
            Err(BackendError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn list_all_keys_recursive(&self, key_prefix: &str) -> Result<Vec<String>, BackendError> {
        let mut keys = Vec::new();
        let mut continuation: Option<String> = None;
        loop {
            let out = retry(
                &self.retry_policy,
                &self.breaker,
                &SystemClock,
                jitter_seed(),
                sleep_ms,
                |_attempt| async {
                    let mut req = self
                        .client
                        .list_objects_v2()
                        .bucket(&self.bucket)
                        .prefix(key_prefix);
                    if let Some(token) = &continuation {
                        req = req.continuation_token(token);
                    }
                    req.send().await.map_err(|e| classify(&e))
                },
            )
            .await?;
            for obj in out.contents() {
                if let Some(k) = obj.key() {
                    keys.push(k.to_string());
                }
            }
            if out.is_truncated().unwrap_or(false) {
                continuation = Some(next_continuation(
                    continuation.as_deref(),
                    out.next_continuation_token(),
                )?);
            } else {
                break;
            }
        }
        Ok(keys)
    }

    async fn delete_keys(&self, keys: &[String]) -> Result<(), BackendError> {
        for chunk in keys.chunks(1000) {
            let ids: Vec<_> = chunk
                .iter()
                .map(|k| {
                    aws_sdk_s3::types::ObjectIdentifier::builder()
                        .key(k)
                        .build()
                        .expect("key always present")
                })
                .collect();
            let delete = aws_sdk_s3::types::Delete::builder()
                .set_objects(Some(ids))
                .build()
                .map_err(|e| {
                    BackendError::Other(format!("s3: DeleteObjects request not valid: {e}"))
                })?;
            let out = retry(
                &self.retry_policy,
                &self.breaker,
                &SystemClock,
                jitter_seed(),
                sleep_ms,
                |_attempt| async {
                    self.client
                        .delete_objects()
                        .bucket(&self.bucket)
                        .delete(delete.clone())
                        .send()
                        .await
                        .map_err(|e| classify(&e))
                },
            )
            .await?;
            if let Some(failed) = out.errors().first() {
                return Err(BackendError::Other(format!(
                    "s3: DeleteObjects did not delete {} key(s) (first: `{}`, {}: {})",
                    out.errors().len(),
                    failed.key().unwrap_or("?"),
                    failed.code().unwrap_or("?"),
                    failed.message().unwrap_or("")
                )));
            }
        }
        Ok(())
    }
}

fn next_continuation(previous: Option<&str>, next: Option<&str>) -> Result<String, BackendError> {
    match next {
        Some(token) if Some(token) != previous => Ok(token.to_string()),
        _ => Err(BackendError::Other(
            "s3: the provider reports more listing pages but gives no new token to \
             request them"
                .to_string(),
        )),
    }
}

#[async_trait]
impl StorageBackend for S3Backend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            atomic_write: true,
            native_rename: false,
            conditional_create: true,
            random_read: true,
            stale_path_cache: false,
        }
    }

    async fn exists(&self, path: &ObjPath) -> Result<bool, BackendError> {
        let key = self.key(path);
        if self.head(&key).await?.is_some() {
            return Ok(true);
        }
        let dir_prefix = list_prefix(&self.prefix, path);
        let out = retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                self.client
                    .list_objects_v2()
                    .bucket(&self.bucket)
                    .prefix(&dir_prefix)
                    .max_keys(1)
                    .send()
                    .await
                    .map_err(|e| classify(&e))
            },
        )
        .await?;
        Ok(!out.contents().is_empty())
    }

    async fn get(&self, path: &ObjPath) -> Result<Vec<u8>, BackendError> {
        let key = self.key(path);
        let out = retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                self.client
                    .get_object()
                    .bucket(&self.bucket)
                    .key(&key)
                    .send()
                    .await
                    .map_err(|e| {
                        if e.as_service_error().is_some_and(
                            aws_sdk_s3::operation::get_object::GetObjectError::is_no_such_key,
                        ) {
                            BackendError::NotFound
                        } else {
                            classify(&e)
                        }
                    })
            },
        )
        .await?;
        out.body
            .collect()
            .await
            .map(aws_sdk_s3::primitives::AggregatedBytes::to_vec)
            .map_err(|e| BackendError::Transient(format!("s3: body read interrupted: {e}")))
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
        let key = self.key(path);
        let range = format!(
            "bytes={offset}-{}",
            offset.saturating_add(len).saturating_sub(1)
        );
        let out = retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                match self
                    .client
                    .get_object()
                    .bucket(&self.bucket)
                    .key(&key)
                    .range(&range)
                    .send()
                    .await
                {
                    Ok(out) => Ok(Some(out)),
                    Err(e) if e.raw_response().map(|r| r.status().as_u16()) == Some(416) => {
                        Ok(None)
                    }
                    Err(e) => Err(
                        if e.as_service_error().is_some_and(
                            aws_sdk_s3::operation::get_object::GetObjectError::is_no_such_key,
                        ) {
                            BackendError::NotFound
                        } else {
                            classify(&e)
                        },
                    ),
                }
            },
        )
        .await?;
        let Some(out) = out else {
            return Ok(Vec::new());
        };
        out.body
            .collect()
            .await
            .map(aws_sdk_s3::primitives::AggregatedBytes::to_vec)
            .map_err(|e| BackendError::Transient(format!("s3: range read interrupted: {e}")))
    }

    async fn stat(&self, path: &ObjPath) -> Result<ObjStat, BackendError> {
        let key = self.key(path);
        let size = self.head(&key).await?.ok_or(BackendError::NotFound)?;
        Ok(ObjStat {
            size: u64::try_from(size.max(0)).unwrap_or(0),
        })
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        let key = self.key(path);
        let owned = Bytes::copy_from_slice(data);
        retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                let mut req = self
                    .client
                    .put_object()
                    .bucket(&self.bucket)
                    .key(&key)
                    .body(ByteStream::from(owned.clone()));
                if let Some(mode) = self.object_lock_mode {
                    req = req.object_lock_mode(mode.to_sdk());
                    if let Some(until) = retain_until_date(self.object_lock_days) {
                        req = req.object_lock_retain_until_date(until);
                    }
                }
                req.send().await.map_err(|e| classify(&e))
            },
        )
        .await?;
        Ok(())
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        let key = self.key(path);
        let out = retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                self.client
                    .get_object()
                    .bucket(&self.bucket)
                    .key(&key)
                    .send()
                    .await
                    .map_err(|e| {
                        if e.as_service_error().is_some_and(
                            aws_sdk_s3::operation::get_object::GetObjectError::is_no_such_key,
                        ) {
                            BackendError::NotFound
                        } else {
                            classify(&e)
                        }
                    })
            },
        )
        .await?;
        Ok(Box::new(S3Reader::new(out.body)))
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        let key = self.key(path);
        Ok(Box::new(S3Writer::new(
            self.client.clone(),
            self.bucket.clone(),
            key,
            self.remote_id.clone(),
            Arc::clone(&self.resume),
            self.retry_policy,
            Arc::clone(&self.breaker),
            false,
            self.object_lock_mode,
            self.object_lock_days,
        )))
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        let key = self.key(path);
        Ok(Box::new(S3Writer::new(
            self.client.clone(),
            self.bucket.clone(),
            key,
            self.remote_id.clone(),
            Arc::clone(&self.resume),
            self.retry_policy,
            Arc::clone(&self.breaker),
            true,
            self.object_lock_mode,
            self.object_lock_days,
        )))
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        let key = self.key(path);
        retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            jitter_seed(),
            sleep_ms,
            |_attempt| async {
                self.client
                    .delete_object()
                    .bucket(&self.bucket)
                    .key(&key)
                    .send()
                    .await
                    .map_err(|e| classify(&e))
            },
        )
        .await?;
        Ok(())
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        let listing_prefix = list_prefix(&self.prefix, dir);
        let mut entries = Vec::new();
        let mut continuation: Option<String> = None;
        let mut saw_anything = false;
        loop {
            let out = retry(
                &self.retry_policy,
                &self.breaker,
                &SystemClock,
                jitter_seed(),
                sleep_ms,
                |_attempt| async {
                    let mut req = self
                        .client
                        .list_objects_v2()
                        .bucket(&self.bucket)
                        .prefix(&listing_prefix)
                        .delimiter("/");
                    if let Some(token) = &continuation {
                        req = req.continuation_token(token);
                    }
                    req.send().await.map_err(|e| classify(&e))
                },
            )
            .await?;

            for obj in out.contents() {
                let Some(k) = obj.key() else { continue };
                if k == listing_prefix {
                    continue;
                }
                saw_anything = true;
                entries.push(ListEntry {
                    name: crate::objkey::last_segment(k).to_string(),
                    kind: EntryKind::File,
                    size: obj.size().map(|s| u64::try_from(s.max(0)).unwrap_or(0)),
                    modified: obj.last_modified().map(aws_smithy_types::DateTime::secs),
                });
            }
            for cp in out.common_prefixes() {
                if let Some(p) = cp.prefix() {
                    saw_anything = true;
                    entries.push(ListEntry {
                        name: crate::objkey::last_segment(p).to_string(),
                        kind: EntryKind::Directory,
                        size: None,
                        modified: None,
                    });
                }
            }

            if out.is_truncated().unwrap_or(false) {
                continuation = Some(next_continuation(
                    continuation.as_deref(),
                    out.next_continuation_token(),
                )?);
            } else {
                break;
            }
        }

        if !saw_anything && !dir.is_root() && !self.exists(dir).await? {
            return Err(BackendError::NotFound);
        }
        Ok(entries)
    }

    async fn create_dir_all(&self, _dir: &ObjPath) -> Result<(), BackendError> {
        Ok(())
    }

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        from_kind: EntryKind,
    ) -> Result<(), BackendError> {
        let from_key = self.key(from);
        let to_key = self.key(to);

        let (source_keys, dest_keys): (Vec<String>, Vec<String>) = match from_kind {
            EntryKind::File => (vec![from_key.clone()], vec![to_key.clone()]),
            EntryKind::Directory => {
                let from_prefix = list_prefix(&self.prefix, from);
                let keys = self.list_all_keys_recursive(&from_prefix).await?;
                let dests = keys
                    .iter()
                    .map(|k| format!("{to_key}/{}", &k[from_prefix.len()..]))
                    .collect();
                (keys, dests)
            }
        };

        if source_keys.is_empty() {
            return Err(BackendError::NotFound);
        }

        for (src, dst) in source_keys.iter().zip(dest_keys.iter()) {
            let copy_source = format!("{}/{}", self.bucket, percent_encode_copy_source(src));
            retry(
                &self.retry_policy,
                &self.breaker,
                &SystemClock,
                jitter_seed(),
                sleep_ms,
                |_attempt| async {
                    let mut req = self
                        .client
                        .copy_object()
                        .bucket(&self.bucket)
                        .key(dst)
                        .copy_source(&copy_source);
                    if let Some(mode) = self.object_lock_mode {
                        req = req.object_lock_mode(mode.to_sdk());
                        if let Some(until) = retain_until_date(self.object_lock_days) {
                            req = req.object_lock_retain_until_date(until);
                        }
                    }
                    req.send().await.map_err(|e| classify(&e))
                },
            )
            .await?;
        }
        self.delete_keys(&source_keys).await
    }

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        emulate_remove_dir_all_via_delete(self, dir).await
    }

    async fn load_resume_prologue(
        &self,
        path: &ObjPath,
        content_hash: Option<[u8; 32]>,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        let key = self.key(path);
        let Some(record) = self.resume.load(&self.remote_id, &key).await? else {
            return Ok(None);
        };
        if record.source_hash != content_hash {
            self.resume.abort(&self.remote_id, &key).await?;
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
        let key = self.key(path);
        let source_hash = content_hash.unwrap_or_else(crate::writer::fresh_source_marker);
        self.resume
            .begin(
                &self.remote_id,
                &key,
                crate::writer::PART_SIZE as u64,
                source_hash,
            )
            .await?;
        self.resume.set_prologue(&self.remote_id, &key, bytes).await
    }
}

fn percent_encode_copy_source(key: &str) -> String {
    cu_backend::percent::encode_path_preserving_slash(key)
}
