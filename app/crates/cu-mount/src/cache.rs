// SPDX-License-Identifier: GPL-3.0-or-later

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use cu_format::content::PLAINTEXT_CHUNK_LEN;
use lru::LruCache;

pub const DEFAULT_BUDGET_BYTES: usize = 256 * 1024 * 1024;

pub type ChunkKey = (Arc<str>, [u8; 12], u64);

pub struct ChunkCache {
    inner: Mutex<LruCache<ChunkKey, Arc<Vec<u8>>>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl ChunkCache {
    #[must_use]
    pub fn new() -> Self {
        Self::with_budget_bytes(DEFAULT_BUDGET_BYTES)
    }

    #[must_use]
    pub fn with_budget_bytes(budget_bytes: usize) -> Self {
        let capacity = (budget_bytes / PLAINTEXT_CHUNK_LEN).max(1);
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(capacity).expect("capacity >= 1 after .max(1)"),
            )),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn get(&self, key: &ChunkKey) -> Option<Arc<Vec<u8>>> {
        let mut inner = self.inner.lock().unwrap();
        let hit = inner.get(key).cloned();
        if hit.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
        }
        hit
    }

    pub fn put(&self, key: ChunkKey, value: Arc<Vec<u8>>) {
        self.inner.lock().unwrap().put(key, value);
    }

    #[must_use]
    pub fn contains(&self, key: &ChunkKey) -> bool {
        self.inner.lock().unwrap().contains(key)
    }

    #[must_use]
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
}

impl Default for ChunkCache {
    fn default() -> Self {
        Self::new()
    }
}
