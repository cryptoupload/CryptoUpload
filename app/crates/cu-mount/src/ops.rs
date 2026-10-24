// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cu_format::config::VaultConfig;
use cu_format::content::PLAINTEXT_CHUNK_LEN;
use cu_vfs::{Entry, FileReader, NodeKind, VaultFs, VfsError};
use tokio::sync::{Semaphore, watch};

use crate::cache::{ChunkCache, ChunkKey, DEFAULT_BUDGET_BYTES};
use crate::inode::{Ino, InodeTable, ROOT_INO};
use crate::readahead::ReadaheadState;

const MAX_CONCURRENT_ON_DEMAND_FETCHES: usize = 8;

const MAX_CONCURRENT_DIR_LOOKUPS: usize = 8;
const MAX_CONCURRENT_PREFETCH_FETCHES: usize = 8;

const MAX_CHUNKS_PER_REQUEST: u64 = 16;

type Inflight = Mutex<HashMap<ChunkKey, watch::Receiver<()>>>;

type Waiting = (u64, Option<watch::Receiver<()>>);

#[derive(Debug)]
pub enum OpError {
    NotFound,
    NotADirectory,
    IsADirectory,
    UnknownHandle,
    Vfs(VfsError),
    Exists,
    NotEmpty,
    ReadOnly,
    Reserved,
    Io(String),
}

impl Coded for OpError {
    fn msg(&self) -> Msg {
        match self {
            Self::NotFound => Msg::new("OP_NOT_FOUND"),
            Self::NotADirectory => Msg::new("OP_NOT_A_DIRECTORY"),
            Self::IsADirectory => Msg::new("OP_IS_A_DIRECTORY"),
            Self::UnknownHandle => Msg::new("OP_UNKNOWN_HANDLE"),
            Self::Vfs(e) => e.msg(),
            Self::Exists => Msg::new("OP_EXISTS"),
            Self::NotEmpty => Msg::new("OP_NOT_EMPTY"),
            Self::ReadOnly => Msg::new("OP_READ_ONLY"),
            Self::Reserved => Msg::new("OP_RESERVED"),
            Self::Io(why) => Msg::new("OP_IO").arg(why),
        }
    }
}

display_as_msg!(OpError);

impl std::error::Error for OpError {}

impl From<crate::staging::StagingError> for OpError {
    fn from(err: crate::staging::StagingError) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<VfsError> for OpError {
    fn from(err: VfsError) -> Self {
        match err {
            VfsError::NotFound => Self::NotFound,
            VfsError::AlreadyExists => Self::Exists,
            other => Self::Vfs(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attr {
    pub kind: NodeKind,
    pub size: u64,
}

pub type Fh = u64;

pub(crate) struct Handle {
    pub(crate) reader: Option<Arc<FileReader>>,
    pub(crate) ino: Ino,
    pub(crate) writable: bool,
    pub(crate) readahead: Mutex<ReadaheadState>,
}

pub type DirSnapshot = Arc<Vec<Entry>>;

pub struct VfsOps {
    pub(crate) fs: Arc<VaultFs>,
    pub(crate) config: VaultConfig,
    pub(crate) inodes: Arc<Mutex<InodeTable>>,
    pub(crate) handles: Mutex<HashMap<Fh, Arc<Handle>>>,
    dir_handles: Mutex<HashMap<Fh, Option<DirSnapshot>>>,
    pub(crate) next_fh: AtomicU64,
    pub(crate) cache: Arc<ChunkCache>,
    prefetch_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    pub(crate) fetch_semaphore: Arc<Semaphore>,
    prefetch_semaphore: Arc<Semaphore>,
    inflight: Arc<Inflight>,
    pub(crate) write: Option<Arc<crate::write::WriteCore>>,
}

impl VfsOps {
    pub async fn new(fs: Arc<VaultFs>, config: VaultConfig) -> Result<Self, OpError> {
        Self::with_cache_budget_bytes(fs, config, DEFAULT_BUDGET_BYTES).await
    }

    pub async fn with_cache_budget_bytes(
        fs: Arc<VaultFs>,
        config: VaultConfig,
        cache_budget_bytes: usize,
    ) -> Result<Self, OpError> {
        let root = fs.resolve_dir_ref(&config, "").await?;
        Ok(Self {
            fs,
            config,
            inodes: Arc::new(Mutex::new(InodeTable::new(root))),
            handles: Mutex::new(HashMap::new()),
            dir_handles: Mutex::new(HashMap::new()),
            next_fh: AtomicU64::new(1),
            cache: Arc::new(ChunkCache::with_budget_bytes(cache_budget_bytes)),
            prefetch_tasks: Mutex::new(Vec::new()),
            fetch_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_ON_DEMAND_FETCHES)),
            prefetch_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_PREFETCH_FETCHES)),
            inflight: Arc::new(Mutex::new(HashMap::new())),
            write: None,
        })
    }

    pub async fn lookup(&self, parent: Ino, name: &str) -> Result<(Ino, Attr), OpError> {
        if parent == ROOT_INO && name == crate::write::RESERVED_DIR {
            return Err(OpError::NotFound);
        }
        if let Some(hit) = self.dirty_lookup(parent, name) {
            return Ok(hit);
        }
        let parent_dir = self.dir_ref(parent)?;
        let kind = match self.fs.kind_in(&self.config, &parent_dir, name).await {
            Ok(kind) => kind,
            Err(VfsError::NotFound) => return Err(OpError::NotFound),
            Err(e) => return Err(e.into()),
        };
        match kind {
            NodeKind::Dir => {
                let child = self
                    .fs
                    .descend_dir_ref(&self.config, &parent_dir, name)
                    .await?;
                let ino = self
                    .inodes
                    .lock()
                    .unwrap()
                    .register_dir(parent, name, child);
                Ok((
                    ino,
                    Attr {
                        kind: NodeKind::Dir,
                        size: 0,
                    },
                ))
            }
            NodeKind::File => {
                let reader = self
                    .fs
                    .open_read_at_in(&self.config, &parent_dir, name)
                    .await?;
                self.seed_tail(&reader);
                let size = reader.plaintext_len();
                let ino = self.inodes.lock().unwrap().register_file(parent, name);
                Ok((
                    ino,
                    Attr {
                        kind: NodeKind::File,
                        size,
                    },
                ))
            }
        }
    }

    pub async fn getattr(&self, ino: Ino) -> Result<Attr, OpError> {
        if let Some(attr) = self.dirty_attr(ino) {
            return Ok(attr);
        }
        if self.is_dir(ino) {
            return Ok(Attr {
                kind: NodeKind::Dir,
                size: 0,
            });
        }
        let (parent, name) = self.file_location(ino)?;
        let parent_dir = self.dir_ref(parent)?;
        let reader = self
            .fs
            .open_read_at_in(&self.config, &parent_dir, &name)
            .await?;
        self.seed_tail(&reader);
        Ok(Attr {
            kind: NodeKind::File,
            size: reader.plaintext_len(),
        })
    }

    pub fn getattr_for_handle(&self, fh: Fh) -> Result<Attr, OpError> {
        let handle = self
            .handles
            .lock()
            .unwrap()
            .get(&fh)
            .cloned()
            .ok_or(OpError::UnknownHandle)?;
        if let Some(attr) = self.dirty_attr(handle.ino) {
            return Ok(attr);
        }
        let size = match &handle.reader {
            Some(r) => r.plaintext_len(),
            None => return Err(OpError::NotFound),
        };
        Ok(Attr {
            kind: NodeKind::File,
            size,
        })
    }

    pub async fn readdir(&self, ino: Ino) -> Result<Vec<Entry>, OpError> {
        let dir = self.dir_ref(ino)?;
        let mut entries = self.fs.list_in(&self.config, &dir).await?;
        if ino == ROOT_INO {
            entries.retain(|e| e.name != crate::write::RESERVED_DIR);
        }
        self.overlay_readdir(ino, &mut entries);
        Ok(entries)
    }

    pub fn opendir(&self) -> Fh {
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
        self.dir_handles.lock().unwrap().insert(fh, None);
        fh
    }

    pub async fn dir_snapshot(
        &self,
        fh: Fh,
        ino: Ino,
        refresh: bool,
    ) -> Result<DirSnapshot, OpError> {
        if !refresh && let Some(Some(snapshot)) = self.dir_handles.lock().unwrap().get(&fh) {
            return Ok(Arc::clone(snapshot));
        }
        let mut entries = self.readdir(ino).await?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let snapshot = Arc::new(entries);
        if let Some(slot) = self.dir_handles.lock().unwrap().get_mut(&fh) {
            *slot = Some(Arc::clone(&snapshot));
        }
        Ok(snapshot)
    }

    pub async fn readdir_with_attrs(
        self: &Arc<Self>,
        ino: Ino,
    ) -> Result<Vec<(String, Attr)>, OpError> {
        let entries = self.readdir(ino).await?;
        let limit = Arc::new(Semaphore::new(MAX_CONCURRENT_DIR_LOOKUPS));
        let mut tasks = tokio::task::JoinSet::new();
        for entry in entries {
            let (ops, limit) = (Arc::clone(self), Arc::clone(&limit));
            tasks.spawn(async move {
                let _permit = limit.acquire_owned().await.ok()?;
                let (_ino, attr) = ops.lookup(ino, &entry.name).await.ok()?;
                Some((entry.name, attr))
            });
        }
        let mut out = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            if let Ok(Some(item)) = joined {
                out.push(item);
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    pub fn releasedir(&self, fh: Fh) {
        self.dir_handles.lock().unwrap().remove(&fh);
    }

    pub async fn open(&self, ino: Ino) -> Result<Fh, OpError> {
        if self.is_dir(ino) {
            return Err(OpError::IsADirectory);
        }
        if self.dirty_attr(ino).is_some() {
            let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
            self.handles.lock().unwrap().insert(
                fh,
                Arc::new(Handle {
                    reader: None,
                    ino,
                    writable: false,
                    readahead: Mutex::new(ReadaheadState::new()),
                }),
            );
            return Ok(fh);
        }
        let (parent, name) = self.file_location(ino)?;
        let parent_dir = self.dir_ref(parent)?;
        let reader = Arc::new(
            self.fs
                .open_read_at_in(&self.config, &parent_dir, &name)
                .await?,
        );
        self.seed_tail(&reader);
        if reader.total_chunks() > 1 {
            self.spawn_prefetch(&reader, 0, 1);
        }
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
        self.handles.lock().unwrap().insert(
            fh,
            Arc::new(Handle {
                reader: Some(reader),
                ino,
                writable: false,
                readahead: Mutex::new(ReadaheadState::new()),
            }),
        );
        Ok(fh)
    }

    pub async fn read(&self, fh: Fh, offset: u64, size: u32) -> Result<Vec<u8>, OpError> {
        let handle = self
            .handles
            .lock()
            .unwrap()
            .get(&fh)
            .cloned()
            .ok_or(OpError::UnknownHandle)?;
        self.load_dirty_range(handle.ino, offset, size).await?;
        if let Some(result) = self.read_dirty(handle.ino, offset, size) {
            return result;
        }
        let reader_arc: Arc<FileReader> = match &handle.reader {
            Some(r) => Arc::clone(r),
            None => self.reader_for_ino(handle.ino).await?,
        };
        let reader = &reader_arc;

        let plaintext_len = reader.plaintext_len();
        if offset >= plaintext_len || size == 0 {
            return Ok(Vec::new());
        }
        let end = offset.saturating_add(u64::from(size)).min(plaintext_len);
        let chunk_len = PLAINTEXT_CHUNK_LEN as u64;
        let first_chunk = offset / chunk_len;
        let last_chunk = (end - 1) / chunk_len;

        let chunks = self
            .load_chunks(&reader_arc, first_chunk, last_chunk)
            .await?;
        let mut chunks: Vec<Option<Arc<Vec<u8>>>> = chunks.into_iter().map(Some).collect();

        let mut out = Vec::with_capacity(usize::try_from(end - offset).unwrap_or(usize::MAX));
        for index in first_chunk..=last_chunk {
            let slot = usize::try_from(index - first_chunk).unwrap_or(usize::MAX);
            let chunk = chunks[slot]
                .take()
                .expect("every index between first_chunk and last_chunk was spawned above");
            let chunk_start = index * chunk_len;
            let start_in_chunk = offset.max(chunk_start) - chunk_start;
            let end_in_chunk = end.min(chunk_start + chunk_len) - chunk_start;
            let start_in_chunk = usize::try_from(start_in_chunk).unwrap_or(usize::MAX);
            let end_in_chunk = usize::try_from(end_in_chunk).unwrap_or(usize::MAX);
            out.extend_from_slice(&chunk[start_in_chunk..end_in_chunk]);
        }

        let (prefetch_from, prefetch_count) = handle
            .readahead
            .lock()
            .unwrap()
            .advance(first_chunk, last_chunk);
        if prefetch_count > 0 {
            self.spawn_prefetch(reader, prefetch_from, prefetch_count);
        }
        Ok(out)
    }

    pub async fn release(&self, fh: Fh) -> Result<(), OpError> {
        let handle = self.handles.lock().unwrap().remove(&fh);
        match handle {
            Some(h) if h.writable => self.release_writer(h.ino).await,
            _ => Ok(()),
        }
    }

    async fn reader_for_ino(&self, ino: Ino) -> Result<Arc<FileReader>, OpError> {
        let (parent, name) = self.file_location(ino)?;
        let dir = self.dir_ref(parent)?;
        let reader = self.fs.open_read_at_in(&self.config, &dir, &name).await?;
        self.seed_tail(&reader);
        Ok(Arc::new(reader))
    }

    pub(crate) fn dir_ref(&self, ino: Ino) -> Result<cu_vfs::DirRef, OpError> {
        self.inodes
            .lock()
            .unwrap()
            .dir_ref(ino)
            .cloned()
            .ok_or(OpError::NotADirectory)
    }

    pub(crate) fn file_location(&self, ino: Ino) -> Result<(Ino, String), OpError> {
        self.inodes
            .lock()
            .unwrap()
            .file_location(ino)
            .map(|(parent, name)| (parent, name.to_string()))
            .ok_or(OpError::NotFound)
    }

    pub(crate) fn is_dir(&self, ino: Ino) -> bool {
        self.inodes.lock().unwrap().is_dir(ino)
    }

    #[must_use]
    pub fn parent_ino(&self, ino: Ino) -> Option<Ino> {
        self.inodes.lock().unwrap().parent(ino)
    }

    pub(crate) fn seed_tail(&self, reader: &FileReader) {
        let (path, nonce) = reader.cache_key();
        if let Some((tail_index, tail_bytes)) = reader.tail() {
            self.cache
                .put((path, nonce, tail_index), Arc::new(tail_bytes.to_vec()));
        }
    }

    async fn load_chunks(
        &self,
        reader: &Arc<FileReader>,
        first: u64,
        last: u64,
    ) -> Result<Vec<Arc<Vec<u8>>>, OpError> {
        let (path, nonce) = reader.cache_key();
        let slot = |index: u64| usize::try_from(index - first).unwrap_or(usize::MAX);
        let count = slot(last) + 1;
        let mut chunks: Vec<Option<Arc<Vec<u8>>>> = vec![None; count];
        let mut misses = Vec::new();
        for index in first..=last {
            match self.cache.get(&(Arc::clone(&path), nonce, index)) {
                Some(hit) => chunks[slot(index)] = Some(hit),
                None => misses.push(index),
            }
        }
        if misses.is_empty() {
            return Ok(chunks.into_iter().flatten().collect());
        }

        let (runs, waiting) = claim(&self.inflight, &self.cache, reader, &misses);
        let mut set = tokio::task::JoinSet::new();
        for run in runs {
            let fs = Arc::clone(&self.fs);
            let cache = Arc::clone(&self.cache);
            let semaphore = Arc::clone(&self.fetch_semaphore);
            let reader = Arc::clone(reader);
            set.spawn(async move {
                let start = run.start;
                let fetched = fetch_run(&fs, &cache, &semaphore, &reader, &run).await;
                drop(run);
                (start, fetched)
            });
        }
        while let Some(joined) = set.join_next().await {
            let (start, fetched) = joined.expect("a chunk fetch must never panic");
            for (offset, chunk) in (0u64..).zip(fetched?) {
                chunks[slot(start + offset)] = Some(chunk);
            }
        }
        for (index, rx) in waiting {
            if let Some(mut rx) = rx {
                let _ = rx.changed().await;
            }
            chunks[slot(index)] = self.cache.get(&(Arc::clone(&path), nonce, index));
        }
        for index in first..=last {
            if chunks[slot(index)].is_none() {
                let _permit = self
                    .fetch_semaphore
                    .acquire()
                    .await
                    .expect("the semaphore is never closed");
                let plaintext = Arc::new(self.fs.read_chunk(reader, index).await?);
                self.cache
                    .put((Arc::clone(&path), nonce, index), Arc::clone(&plaintext));
                chunks[slot(index)] = Some(plaintext);
            }
        }
        Ok(chunks.into_iter().flatten().collect())
    }

    fn spawn_prefetch(&self, reader: &Arc<FileReader>, from: u64, count: u64) {
        let total = reader.total_chunks();
        let end = from.saturating_add(count).min(total);
        let mut tasks = self.prefetch_tasks.lock().unwrap();
        tasks.retain(|t| !t.is_finished());
        let mut group_start = from;
        while group_start < end {
            let group_end = group_start.saturating_add(MAX_CHUNKS_PER_REQUEST).min(end);
            let ops = PrefetchCtx {
                fs: Arc::clone(&self.fs),
                cache: Arc::clone(&self.cache),
                semaphore: Arc::clone(&self.prefetch_semaphore),
                inflight: Arc::clone(&self.inflight),
            };
            let reader = Arc::clone(reader);
            tasks.push(tokio::spawn(async move {
                ops.run(&reader, group_start, group_end).await;
            }));
            group_start = group_end;
        }
    }
}

struct Claim {
    inflight: Arc<Inflight>,
    keys: Vec<ChunkKey>,
    start: u64,
    len: u64,
    rx: watch::Receiver<()>,
    _tx: watch::Sender<()>,
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut map = self
            .inflight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for key in &self.keys {
            if map.get(key).is_some_and(|rx| rx.same_channel(&self.rx)) {
                map.remove(key);
            }
        }
    }
}

fn claim(
    inflight: &Arc<Inflight>,
    cache: &ChunkCache,
    reader: &FileReader,
    indices: &[u64],
) -> (Vec<Claim>, Vec<Waiting>) {
    let (path, nonce) = reader.cache_key();
    let mut map = inflight.lock().unwrap();
    let mut runs: Vec<Claim> = Vec::new();
    let mut waiting = Vec::new();
    for &index in indices {
        let key = (Arc::clone(&path), nonce, index);
        if let Some(rx) = map.get(&key) {
            waiting.push((index, Some(rx.clone())));
            continue;
        }
        if cache.contains(&key) {
            waiting.push((index, None));
            continue;
        }
        let extend = runs
            .last()
            .is_some_and(|r| r.start + r.len == index && r.len < MAX_CHUNKS_PER_REQUEST);
        if !extend {
            let (tx, rx) = watch::channel(());
            runs.push(Claim {
                inflight: Arc::clone(inflight),
                keys: Vec::new(),
                start: index,
                len: 0,
                rx,
                _tx: tx,
            });
        }
        let run = runs.last_mut().expect("just inserted");
        map.insert(key.clone(), run.rx.clone());
        run.keys.push(key);
        run.len += 1;
    }
    (runs, waiting)
}

async fn fetch_run(
    fs: &VaultFs,
    cache: &ChunkCache,
    semaphore: &Semaphore,
    reader: &FileReader,
    run: &Claim,
) -> Result<Vec<Arc<Vec<u8>>>, OpError> {
    let _permit = semaphore
        .acquire()
        .await
        .expect("the semaphore is never closed");
    let plaintexts = fs.read_chunks(reader, run.start, run.len).await?;
    let mut out = Vec::with_capacity(plaintexts.len());
    for (key, plaintext) in run.keys.iter().zip(plaintexts) {
        let plaintext = Arc::new(plaintext);
        cache.put(key.clone(), Arc::clone(&plaintext));
        out.push(plaintext);
    }
    Ok(out)
}

struct PrefetchCtx {
    fs: Arc<VaultFs>,
    cache: Arc<ChunkCache>,
    semaphore: Arc<Semaphore>,
    inflight: Arc<Inflight>,
}

impl PrefetchCtx {
    async fn run(&self, reader: &FileReader, from: u64, end: u64) {
        let Ok(_permit) = self.semaphore.acquire().await else {
            return;
        };
        let indices: Vec<u64> = (from..end).collect();
        let (claims, _already_in_flight) = claim(&self.inflight, &self.cache, reader, &indices);
        for claim in claims {
            let Ok(plaintexts) = self.fs.read_chunks(reader, claim.start, claim.len).await else {
                continue;
            };
            for (key, plaintext) in claim.keys.iter().zip(plaintexts) {
                self.cache.put(key.clone(), Arc::new(plaintext));
            }
        }
    }
}
