// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cu_backend::BackendError;
use cu_backend::resume::{PartRecord, ResumeRecord, ResumeStore};
use cu_index::{Db, RemoteUploadPartRow, RemoteUploadRow};

use crate::error::CoreError;

#[must_use]
pub fn remotes_state_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join("remotes")
}

#[must_use]
pub fn remote_state_db_path(data_dir: &Path, remote_id: &str) -> PathBuf {
    remotes_state_dir(data_dir).join(format!("{remote_id}.db"))
}

pub struct SqliteResumeStore {
    db: Arc<Mutex<Db>>,
}

impl SqliteResumeStore {
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        Ok(Self {
            db: Arc::new(Mutex::new(Db::open(path)?)),
        })
    }

    async fn blocking<T, F>(&self, f: F) -> Result<T, BackendError>
    where
        F: FnOnce(&Db) -> Result<T, BackendError> + Send + 'static,
        T: Send + 'static,
    {
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || {
            let db = db.lock().unwrap();
            f(&db)
        })
        .await
        .unwrap_or_else(|e| {
            Err(BackendError::Other(format!(
                "resume local state: task interrupted: {e}"
            )))
        })
    }
}

fn to_backend_err(e: &cu_index::IndexError) -> BackendError {
    BackendError::Other(format!("resume local state: {e}"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn row_to_record(row: RemoteUploadRow, parts: Vec<RemoteUploadPartRow>) -> ResumeRecord {
    ResumeRecord {
        upload_id: row.upload_id,
        part_size: row.part_size,
        prologue: row.prologue,
        source_hash: Some(row.source_hash),
        parts: parts
            .into_iter()
            .map(|p| PartRecord {
                part_number: p.part_number,
                etag: p.etag,
                crc32c: p.crc32c,
                len: p.len,
            })
            .collect(),
    }
}

#[async_trait]
impl ResumeStore for SqliteResumeStore {
    async fn load(
        &self,
        remote_id: &str,
        object_key: &str,
    ) -> Result<Option<ResumeRecord>, BackendError> {
        let remote_id = remote_id.to_string();
        let object_key = object_key.to_string();
        self.blocking(move |db| {
            let Some(row) = db
                .resume_load(&remote_id, &object_key)
                .map_err(|e| to_backend_err(&e))?
            else {
                return Ok(None);
            };
            let parts = db
                .resume_parts(row.upload_row_id)
                .map_err(|e| to_backend_err(&e))?;
            Ok(Some(row_to_record(row, parts)))
        })
        .await
    }

    async fn begin(
        &self,
        remote_id: &str,
        object_key: &str,
        part_size: u64,
        source_hash: [u8; 32],
    ) -> Result<(), BackendError> {
        let remote_id = remote_id.to_string();
        let object_key = object_key.to_string();
        self.blocking(move |db| {
            db.resume_begin(&remote_id, &object_key, part_size, source_hash, now_ms())
                .map_err(|e| to_backend_err(&e))
        })
        .await
    }

    async fn set_upload_id(
        &self,
        remote_id: &str,
        object_key: &str,
        upload_id: &str,
    ) -> Result<(), BackendError> {
        let remote_id = remote_id.to_string();
        let object_key = object_key.to_string();
        let upload_id = upload_id.to_string();
        self.blocking(move |db| {
            db.resume_set_upload_id(&remote_id, &object_key, &upload_id, now_ms())
                .map_err(|e| to_backend_err(&e))
        })
        .await
    }

    async fn set_prologue(
        &self,
        remote_id: &str,
        object_key: &str,
        prologue: &[u8],
    ) -> Result<(), BackendError> {
        let remote_id = remote_id.to_string();
        let object_key = object_key.to_string();
        let prologue = prologue.to_vec();
        self.blocking(move |db| {
            db.resume_set_prologue(&remote_id, &object_key, &prologue, now_ms())
                .map_err(|e| to_backend_err(&e))
        })
        .await
    }

    async fn record_part(
        &self,
        remote_id: &str,
        object_key: &str,
        part: PartRecord,
    ) -> Result<(), BackendError> {
        let remote_id = remote_id.to_string();
        let object_key = object_key.to_string();
        self.blocking(move |db| {
            let row = db
                .resume_load(&remote_id, &object_key)
                .map_err(|e| to_backend_err(&e))?
                .ok_or_else(|| {
                    BackendError::Other(format!(
                        "resume_record_part: no open upload for {object_key}"
                    ))
                })?;
            db.resume_record_part(
                row.upload_row_id,
                part.part_number,
                &part.etag,
                part.crc32c,
                part.len,
                now_ms(),
            )
            .map_err(|e| to_backend_err(&e))
        })
        .await
    }

    async fn finish(&self, remote_id: &str, object_key: &str) -> Result<(), BackendError> {
        let remote_id = remote_id.to_string();
        let object_key = object_key.to_string();
        self.blocking(move |db| {
            db.resume_finish(&remote_id, &object_key)
                .map_err(|e| to_backend_err(&e))
        })
        .await
    }

    async fn abort(&self, remote_id: &str, object_key: &str) -> Result<(), BackendError> {
        let remote_id = remote_id.to_string();
        let object_key = object_key.to_string();
        self.blocking(move |db| {
            db.resume_abort(&remote_id, &object_key)
                .map_err(|e| to_backend_err(&e))
        })
        .await
    }

    async fn list_stale(
        &self,
        remote_id: &str,
        older_than_ms: i64,
    ) -> Result<Vec<(String, Option<String>)>, BackendError> {
        let remote_id = remote_id.to_string();
        self.blocking(move |db| {
            db.resume_list_stale(&remote_id, older_than_ms)
                .map_err(|e| to_backend_err(&e))
        })
        .await
    }
}
