// SPDX-License-Identifier: GPL-3.0-or-later

pub mod error;
pub mod http;
pub mod objpath;
pub mod percent;
pub mod proxy;
pub mod resume;
pub mod retry;
mod rng;
pub mod spool;
pub mod throttle;
pub mod timefmt;

pub use cu_msg::{Coded, Msg};
pub use error::{BackendError, is_transient_http_status};
pub use objpath::ObjPath;
pub use resume::{NullResumeStore, PartRecord, ResumeRecord, ResumeStore};
pub use retry::{
    CircuitBreaker, Clock, RetryFailure, RetryPolicy, SystemClock, TokenBucket, jitter_seed, retry,
    retry_with_hint,
};
pub use throttle::{RateLimiter, ThrottleBackend};

use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Capabilities {
    pub atomic_write: bool,
    pub native_rename: bool,
    pub conditional_create: bool,
    pub random_read: bool,
    pub stale_path_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjStat {
    pub size: u64,
}

#[async_trait]
pub trait StreamReader: Send {
    async fn read_chunk(&mut self, max_len: usize) -> Result<Vec<u8>, BackendError>;
}

#[async_trait]
pub trait StreamWriter: Send {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError>;

    async fn commit(self: Box<Self>) -> Result<(), BackendError>;
}

#[async_trait]
pub trait StorageBackend: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    async fn exists(&self, path: &ObjPath) -> Result<bool, BackendError>;

    async fn get(&self, path: &ObjPath) -> Result<Vec<u8>, BackendError>;

    async fn get_range(
        &self,
        path: &ObjPath,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        let data = self.get(path).await?;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(data.len());
        let want = usize::try_from(len).unwrap_or(usize::MAX);
        let end = start.saturating_add(want).min(data.len());
        Ok(data[start..end].to_vec())
    }

    async fn stat(&self, path: &ObjPath) -> Result<ObjStat, BackendError> {
        let data = self.get(path).await?;
        Ok(ObjStat {
            size: u64::try_from(data.len()).unwrap_or(u64::MAX),
        })
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError>;

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError>;

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError>;

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        let _ = path;
        Err(BackendError::Coded(cu_msg::Msg::new(
            "BACKEND_NO_EXCLUSIVE_WRITE",
        )))
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError>;

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError>;

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError>;

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        from_kind: EntryKind,
    ) -> Result<(), BackendError>;

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError>;

    async fn load_resume_prologue(
        &self,
        path: &ObjPath,
        content_hash: Option<[u8; 32]>,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        let _ = (path, content_hash);
        Ok(None)
    }

    async fn store_resume_prologue(
        &self,
        path: &ObjPath,
        bytes: &[u8],
        content_hash: Option<[u8; 32]>,
    ) -> Result<(), BackendError> {
        let _ = (path, bytes, content_hash);
        Ok(())
    }
}

#[async_trait]
impl StorageBackend for Box<dyn StorageBackend> {
    fn capabilities(&self) -> Capabilities {
        (**self).capabilities()
    }

    async fn exists(&self, path: &ObjPath) -> Result<bool, BackendError> {
        (**self).exists(path).await
    }

    async fn get(&self, path: &ObjPath) -> Result<Vec<u8>, BackendError> {
        (**self).get(path).await
    }

    async fn get_range(
        &self,
        path: &ObjPath,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        (**self).get_range(path, offset, len).await
    }

    async fn stat(&self, path: &ObjPath) -> Result<ObjStat, BackendError> {
        (**self).stat(path).await
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        (**self).put(path, data).await
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        (**self).open_read(path).await
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        (**self).open_write(path).await
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        (**self).open_write_exclusive(path).await
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        (**self).delete(path).await
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        (**self).list(dir).await
    }

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        (**self).create_dir_all(dir).await
    }

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        from_kind: EntryKind,
    ) -> Result<(), BackendError> {
        (**self).rename(from, to, from_kind).await
    }

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        (**self).remove_dir_all(dir).await
    }

    async fn load_resume_prologue(
        &self,
        path: &ObjPath,
        content_hash: Option<[u8; 32]>,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        (**self).load_resume_prologue(path, content_hash).await
    }

    async fn store_resume_prologue(
        &self,
        path: &ObjPath,
        bytes: &[u8],
        content_hash: Option<[u8; 32]>,
    ) -> Result<(), BackendError> {
        (**self)
            .store_resume_prologue(path, bytes, content_hash)
            .await
    }
}

pub async fn emulate_remove_dir_all_via_delete(
    backend: &dyn StorageBackend,
    dir: &ObjPath,
) -> Result<(), BackendError> {
    let entries = match backend.list(dir).await {
        Ok(entries) => entries,
        Err(BackendError::NotFound) => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let child = dir.join(&entry.name)?;
        match entry.kind {
            EntryKind::File => backend.delete(&child).await?,
            EntryKind::Directory => {
                Box::pin(emulate_remove_dir_all_via_delete(backend, &child)).await?;
            }
        }
    }
    Ok(())
}

pub async fn emulate_rename_via_copy_and_delete(
    backend: &dyn StorageBackend,
    from: &ObjPath,
    to: &ObjPath,
    from_kind: EntryKind,
) -> Result<(), BackendError> {
    match from_kind {
        EntryKind::File => copy_object(backend, from, to).await?,
        EntryKind::Directory => copy_dir_recursive(backend, from, to).await?,
    }
    delete_recursive(backend, from, from_kind).await
}

async fn copy_object(
    backend: &dyn StorageBackend,
    from: &ObjPath,
    to: &ObjPath,
) -> Result<(), BackendError> {
    const COPY_CHUNK: usize = 256 * 1024;
    let mut reader = backend.open_read(from).await?;
    let mut writer = backend.open_write_exclusive(to).await?;
    loop {
        let chunk = reader.read_chunk(COPY_CHUNK).await?;
        if chunk.is_empty() {
            break;
        }
        writer.write_chunk(&chunk).await?;
    }
    writer.commit().await
}

fn copy_dir_recursive<'a>(
    backend: &'a dyn StorageBackend,
    from: &'a ObjPath,
    to: &'a ObjPath,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), BackendError>> + Send + 'a>> {
    Box::pin(async move {
        backend.create_dir_all(to).await?;
        for entry in backend.list(from).await? {
            let child_from = from.join(&entry.name)?;
            let child_to = to.join(&entry.name)?;
            match entry.kind {
                EntryKind::File => copy_object(backend, &child_from, &child_to).await?,
                EntryKind::Directory => {
                    copy_dir_recursive(backend, &child_from, &child_to).await?;
                }
            }
        }
        Ok(())
    })
}

async fn write_probe_once(
    backend: &dyn StorageBackend,
    path: &ObjPath,
) -> Result<(), BackendError> {
    let mut writer = backend.open_write_exclusive(path).await?;
    writer.write_chunk(b"probe").await?;
    writer.commit().await
}

pub async fn probe_exclusive_create(backend: &dyn StorageBackend) -> Result<bool, BackendError> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = ObjPath::new(format!(
        ".cryptoupload-probe-{:x}-{nanos:x}",
        std::process::id()
    ))?;
    let first = write_probe_once(backend, &path).await;
    let outcome = match first {
        Ok(()) => match write_probe_once(backend, &path).await {
            Err(BackendError::AlreadyExists) => Ok(true),
            Ok(()) => Ok(false),
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };
    let _ = backend.delete(&path).await;
    outcome
}

fn delete_recursive<'a>(
    backend: &'a dyn StorageBackend,
    target: &'a ObjPath,
    target_kind: EntryKind,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), BackendError>> + Send + 'a>> {
    Box::pin(async move {
        if target_kind == EntryKind::File {
            return backend.delete(target).await;
        }
        for entry in backend.list(target).await? {
            let child = target.join(&entry.name)?;
            match entry.kind {
                EntryKind::File => backend.delete(&child).await?,
                EntryKind::Directory => {
                    delete_recursive(backend, &child, EntryKind::Directory).await?;
                }
            }
        }
        Ok(())
    })
}
