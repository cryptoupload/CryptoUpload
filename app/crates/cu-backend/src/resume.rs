// SPDX-License-Identifier: GPL-3.0-or-later

use async_trait::async_trait;

use crate::error::BackendError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartRecord {
    pub part_number: u32,
    pub etag: String,
    pub crc32c: [u8; 4],
    pub len: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeRecord {
    pub upload_id: Option<String>,
    pub part_size: u64,
    pub prologue: Option<Vec<u8>>,
    pub source_hash: Option<[u8; 32]>,
    pub parts: Vec<PartRecord>,
}

#[async_trait]
pub trait ResumeStore: Send + Sync {
    async fn load(
        &self,
        remote_id: &str,
        object_key: &str,
    ) -> Result<Option<ResumeRecord>, BackendError>;

    async fn begin(
        &self,
        remote_id: &str,
        object_key: &str,
        part_size: u64,
        source_hash: [u8; 32],
    ) -> Result<(), BackendError>;

    async fn set_upload_id(
        &self,
        remote_id: &str,
        object_key: &str,
        upload_id: &str,
    ) -> Result<(), BackendError>;

    async fn set_prologue(
        &self,
        remote_id: &str,
        object_key: &str,
        prologue: &[u8],
    ) -> Result<(), BackendError>;

    async fn record_part(
        &self,
        remote_id: &str,
        object_key: &str,
        part: PartRecord,
    ) -> Result<(), BackendError>;

    async fn finish(&self, remote_id: &str, object_key: &str) -> Result<(), BackendError>;

    async fn abort(&self, remote_id: &str, object_key: &str) -> Result<(), BackendError>;

    async fn list_stale(
        &self,
        remote_id: &str,
        older_than_ms: i64,
    ) -> Result<Vec<(String, Option<String>)>, BackendError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NullResumeStore;

#[async_trait]
impl ResumeStore for NullResumeStore {
    async fn load(
        &self,
        _remote_id: &str,
        _object_key: &str,
    ) -> Result<Option<ResumeRecord>, BackendError> {
        Ok(None)
    }

    async fn begin(
        &self,
        _remote_id: &str,
        _object_key: &str,
        _part_size: u64,
        _source_hash: [u8; 32],
    ) -> Result<(), BackendError> {
        Ok(())
    }

    async fn set_upload_id(
        &self,
        _remote_id: &str,
        _object_key: &str,
        _upload_id: &str,
    ) -> Result<(), BackendError> {
        Ok(())
    }

    async fn set_prologue(
        &self,
        _remote_id: &str,
        _object_key: &str,
        _prologue: &[u8],
    ) -> Result<(), BackendError> {
        Ok(())
    }

    async fn record_part(
        &self,
        _remote_id: &str,
        _object_key: &str,
        _part: PartRecord,
    ) -> Result<(), BackendError> {
        Ok(())
    }

    async fn finish(&self, _remote_id: &str, _object_key: &str) -> Result<(), BackendError> {
        Ok(())
    }

    async fn abort(&self, _remote_id: &str, _object_key: &str) -> Result<(), BackendError> {
        Ok(())
    }

    async fn list_stale(
        &self,
        _remote_id: &str,
        _older_than_ms: i64,
    ) -> Result<Vec<(String, Option<String>)>, BackendError> {
        Ok(Vec::new())
    }
}
