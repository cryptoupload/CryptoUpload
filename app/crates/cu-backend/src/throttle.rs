// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::error::BackendError;
use crate::objpath::ObjPath;
use crate::{Capabilities, EntryKind, ListEntry, StorageBackend, StreamReader, StreamWriter};

pub struct RateLimiter {
    bytes_per_sec: f64,
    capacity: f64,
    state: Mutex<(f64, Instant)>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(bytes_per_sec: u64) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let bytes_per_sec = bytes_per_sec.max(1) as f64;
        Self {
            bytes_per_sec,
            capacity: bytes_per_sec,
            state: Mutex::new((bytes_per_sec, Instant::now())),
        }
    }

    pub async fn acquire(&self, bytes: usize) {
        #[allow(clippy::cast_precision_loss)]
        let mut remaining = bytes as f64;
        loop {
            let wait = {
                let mut guard = self.state.lock().unwrap();
                let (tokens, last) = &mut *guard;
                let elapsed = last.elapsed().as_secs_f64();
                *tokens = (*tokens + elapsed * self.bytes_per_sec).min(self.capacity);
                *last = Instant::now();
                if *tokens >= remaining {
                    *tokens -= remaining;
                    None
                } else {
                    let deficit = remaining - *tokens;
                    *tokens = 0.0;
                    remaining = deficit;
                    Some(Duration::from_secs_f64(deficit / self.bytes_per_sec))
                }
            };
            match wait {
                None => return,
                Some(d) => tokio::time::sleep(d).await,
            }
        }
    }
}

pub struct ThrottleBackend<B> {
    inner: B,
    limiter: std::sync::Arc<RateLimiter>,
}

impl<B: StorageBackend> ThrottleBackend<B> {
    #[must_use]
    pub fn new(inner: B, limiter: std::sync::Arc<RateLimiter>) -> Self {
        Self { inner, limiter }
    }
}

#[async_trait]
impl<B: StorageBackend> StorageBackend for ThrottleBackend<B> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn exists(&self, path: &ObjPath) -> Result<bool, BackendError> {
        self.inner.exists(path).await
    }

    async fn get(&self, path: &ObjPath) -> Result<Vec<u8>, BackendError> {
        self.inner.get(path).await
    }

    async fn get_range(
        &self,
        path: &ObjPath,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        self.inner.get_range(path, offset, len).await
    }

    async fn stat(&self, path: &ObjPath) -> Result<crate::ObjStat, BackendError> {
        self.inner.stat(path).await
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        self.inner.put(path, data).await
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        let inner = self.inner.open_read(path).await?;
        Ok(Box::new(ThrottledReader {
            inner,
            limiter: std::sync::Arc::clone(&self.limiter),
        }))
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        let inner = self.inner.open_write(path).await?;
        Ok(Box::new(ThrottledWriter {
            inner,
            limiter: std::sync::Arc::clone(&self.limiter),
        }))
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        let inner = self.inner.open_write_exclusive(path).await?;
        Ok(Box::new(ThrottledWriter {
            inner,
            limiter: std::sync::Arc::clone(&self.limiter),
        }))
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        self.inner.delete(path).await
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        self.inner.list(dir).await
    }

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        self.inner.create_dir_all(dir).await
    }

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        from_kind: EntryKind,
    ) -> Result<(), BackendError> {
        self.inner.rename(from, to, from_kind).await
    }

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        self.inner.remove_dir_all(dir).await
    }

    async fn load_resume_prologue(
        &self,
        path: &ObjPath,
        content_hash: Option<[u8; 32]>,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        self.inner.load_resume_prologue(path, content_hash).await
    }

    async fn store_resume_prologue(
        &self,
        path: &ObjPath,
        bytes: &[u8],
        content_hash: Option<[u8; 32]>,
    ) -> Result<(), BackendError> {
        self.inner
            .store_resume_prologue(path, bytes, content_hash)
            .await
    }
}

struct ThrottledReader {
    inner: Box<dyn StreamReader>,
    limiter: std::sync::Arc<RateLimiter>,
}

#[async_trait]
impl StreamReader for ThrottledReader {
    async fn read_chunk(&mut self, max_len: usize) -> Result<Vec<u8>, BackendError> {
        let chunk = self.inner.read_chunk(max_len).await?;
        if !chunk.is_empty() {
            self.limiter.acquire(chunk.len()).await;
        }
        Ok(chunk)
    }
}

struct ThrottledWriter {
    inner: Box<dyn StreamWriter>,
    limiter: std::sync::Arc<RateLimiter>,
}

#[async_trait]
impl StreamWriter for ThrottledWriter {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        if !data.is_empty() {
            self.limiter.acquire(data.len()).await;
        }
        self.inner.write_chunk(data).await
    }

    async fn commit(self: Box<Self>) -> Result<(), BackendError> {
        self.inner.commit().await
    }
}
