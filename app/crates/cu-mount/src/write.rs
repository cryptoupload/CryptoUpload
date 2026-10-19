// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cu_crypto::aead;
use cu_format::common::SeedId;
use cu_format::config::VaultConfig;
use cu_format::content::PLAINTEXT_CHUNK_LEN;
use cu_format::keys;
use cu_vfs::{Entry, FileReader, NodeKind, VaultFs};

use crate::inode::{Ino, InodeTable};
use crate::ops::{Attr, Fh, Handle, OpError, VfsOps};
use crate::readahead::ReadaheadState;
use crate::staging::StagingFile;

pub(crate) const RESERVED_DIR: &str = ".cryptoupload";

pub type HookFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub trait MountWriteHooks: Send + Sync {
    fn trash<'a>(&'a self, vault_path: &'a str) -> HookFuture<'a>;
}

#[derive(Clone)]
pub struct WriteOptions {
    pub staging_dir: PathBuf,
    pub hooks: Option<Arc<dyn MountWriteHooks>>,
}

impl std::fmt::Debug for WriteOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteOptions")
            .field("staging_dir", &self.staging_dir)
            .field("trash", &self.hooks.is_some())
            .finish()
    }
}

struct LazyBase {
    reader: FileReader,
    resolved: tokio::sync::Mutex<Vec<bool>>,
    pending: AtomicBool,
}

pub(crate) struct DirtyFile {
    staging: Arc<StagingFile>,
    lazy: Option<LazyBase>,
    path: PathBuf,
    journal: PathBuf,
    existed: AtomicBool,
    writers: AtomicUsize,
    modified: AtomicBool,
    unlinked: AtomicBool,
    journaled: AtomicBool,
    generation: AtomicU64,
    commit_lock: tokio::sync::Mutex<()>,
}

impl Drop for DirtyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.journal);
    }
}

pub const DEFAULT_UPLOAD_DELAY: Duration = Duration::from_secs(2);

const MAX_UPLOAD_ATTEMPTS: u32 = 8;
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const JOURNAL_SUFFIX: &str = ".jrn";
const JOURNAL_AAD: &[u8] = b"cu-mount-journal-v1";
const STAGING_CHUNKS_PER_REQUEST: u64 = 64;

pub struct WriteCore {
    fs: Arc<VaultFs>,
    config: VaultConfig,
    inodes: Arc<Mutex<InodeTable>>,
    staging_dir: PathBuf,
    hooks: Option<Arc<dyn MountWriteHooks>>,
    dirty: Mutex<HashMap<Ino, Arc<DirtyFile>>>,
    materialize: tokio::sync::Mutex<()>,
    counter: AtomicU64,
    upload_delay: Mutex<Duration>,
}

fn reserved(path: &str) -> bool {
    path == RESERVED_DIR || path.starts_with(".cryptoupload/")
}

fn plain_name(name: &str) -> Result<(), OpError> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(OpError::Io(format!("invalid name: \"{name}\"")));
    }
    Ok(())
}

fn split_path(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

#[allow(clippy::unwrap_used)]
impl WriteCore {
    fn dirty_get(&self, ino: Ino) -> Option<Arc<DirtyFile>> {
        self.dirty.lock().unwrap().get(&ino).cloned()
    }

    fn forget_dirty(&self, ino: Ino) {
        self.dirty.lock().unwrap().remove(&ino);
    }

    fn all_dirty(&self) -> Vec<(Ino, Arc<DirtyFile>)> {
        self.dirty
            .lock()
            .unwrap()
            .iter()
            .map(|(i, d)| (*i, Arc::clone(d)))
            .collect()
    }

    fn file_location(&self, ino: Ino) -> Result<(Ino, String), OpError> {
        self.inodes
            .lock()
            .unwrap()
            .file_location(ino)
            .map(|(parent, name)| (parent, name.to_string()))
            .ok_or(OpError::NotFound)
    }

    fn dir_ref(&self, ino: Ino) -> Result<cu_vfs::DirRef, OpError> {
        self.inodes
            .lock()
            .unwrap()
            .dir_ref(ino)
            .cloned()
            .ok_or(OpError::NotADirectory)
    }

    fn is_dir(&self, ino: Ino) -> bool {
        self.inodes.lock().unwrap().is_dir(ino)
    }

    fn child_path(&self, parent: Ino, name: &str) -> Result<String, OpError> {
        plain_name(name)?;
        let base = self
            .inodes
            .lock()
            .unwrap()
            .path_of(parent)
            .ok_or(OpError::NotADirectory)?;
        let path = if base.is_empty() {
            name.to_string()
        } else {
            format!("{base}/{name}")
        };
        if reserved(&path) {
            return Err(OpError::Reserved);
        }
        Ok(path)
    }

    fn new_staging_path(&self) -> PathBuf {
        loop {
            let n = self.counter.fetch_add(1, Ordering::Relaxed);
            let path = self
                .staging_dir
                .join(format!("{}-{n}.stg", std::process::id()));
            let mut journal = path.clone().into_os_string();
            journal.push(JOURNAL_SUFFIX);
            if !path.exists() && !PathBuf::from(journal).exists() {
                return path;
            }
        }
    }

    fn journal_path(staging: &Path) -> PathBuf {
        let mut p = staging.to_path_buf().into_os_string();
        p.push(JOURNAL_SUFFIX);
        PathBuf::from(p)
    }

    async fn trash_or_delete(&self, path: &str) -> Result<(), OpError> {
        if let Some(hooks) = &self.hooks {
            return hooks.trash(path).await.map_err(OpError::Io);
        }
        match self.fs.remove_file(&self.config, path).await {
            Ok(()) | Err(cu_vfs::VfsError::NotFound) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn write_journal(&self, ino: Ino, d: &DirtyFile) -> Result<(), OpError> {
        let (parent, name) = self.file_location(ino)?;
        let path = self.child_path(parent, &name)?;
        d.staging.fsync()?;
        let mut payload = Vec::with_capacity(9 + path.len());
        payload.extend_from_slice(&d.staging.len().to_be_bytes());
        payload.push(u8::from(d.existed.load(Ordering::Acquire)));
        payload.extend_from_slice(path.as_bytes());
        let key = keys::header_key(self.config.latest_seed(), self.config.kdf_salt())
            .map_err(|e| OpError::Io(e.to_string()))?;
        let sealed = aead::seal_with_aad(&key, JOURNAL_AAD, &payload)
            .map_err(|_| OpError::Io("journal: encryption failed".into()))?;
        let mut out = Vec::with_capacity(4 + sealed.len());
        out.extend_from_slice(self.config.latest_seed_id().as_bytes());
        out.extend_from_slice(&sealed);
        let tmp = d.journal.with_extension("jrn.tmp");
        std::fs::write(&tmp, &out).map_err(|e| OpError::Io(e.to_string()))?;
        std::fs::rename(&tmp, &d.journal).map_err(|e| OpError::Io(e.to_string()))?;
        d.journaled.store(true, Ordering::Release);
        Ok(())
    }

    fn refresh_journal(&self, ino: Ino) {
        if let Some(d) = self.dirty_get(ino)
            && d.journaled.load(Ordering::Acquire)
        {
            let _ = self.write_journal(ino, &d);
        }
    }

    fn open_journal(&self, bytes: &[u8]) -> Option<(u64, bool, String)> {
        let (id, sealed) = bytes.split_first_chunk::<4>()?;
        let seed = self.config.seed(SeedId(*id))?;
        let key = keys::header_key(seed, self.config.kdf_salt()).ok()?;
        let payload = aead::open_with_aad(&key, JOURNAL_AAD, sealed).ok()?;
        let (len, rest) = payload.split_first_chunk::<8>()?;
        let (existed, path) = rest.split_first()?;
        Some((
            u64::from_be_bytes(*len),
            *existed != 0,
            String::from_utf8(path.to_vec()).ok()?,
        ))
    }

    async fn load_chunks(&self, d: &DirtyFile, first: u64, last: u64) -> Result<(), OpError> {
        let Some(lazy) = &d.lazy else {
            return Ok(());
        };
        if !lazy.pending.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut resolved = lazy.resolved.lock().await;
        let last = last.min((resolved.len() as u64).saturating_sub(1));
        let unresolved = |resolved: &[bool], index: u64| {
            usize::try_from(index)
                .ok()
                .and_then(|i| resolved.get(i))
                .is_some_and(|r| !*r)
        };
        let mut index = first;
        while index <= last {
            if !unresolved(&resolved, index) {
                index += 1;
                continue;
            }
            let mut run_end = index + 1;
            while run_end <= last
                && run_end - index < STAGING_CHUNKS_PER_REQUEST
                && unresolved(&resolved, run_end)
            {
                run_end += 1;
            }
            let plains = self
                .fs
                .read_chunks(&lazy.reader, index, run_end - index)
                .await?;
            for (chunk_index, plain) in (index..).zip(plains) {
                d.staging
                    .write_at(chunk_index * PLAINTEXT_CHUNK_LEN as u64, &plain)?;
                if let Some(slot) = usize::try_from(chunk_index)
                    .ok()
                    .and_then(|i| resolved.get_mut(i))
                {
                    *slot = true;
                }
            }
            index = run_end;
        }
        if resolved.iter().all(|r| *r) {
            lazy.pending.store(false, Ordering::Release);
        }
        Ok(())
    }

    async fn load_range(&self, d: &DirtyFile, offset: u64, size: u64) -> Result<(), OpError> {
        let end = offset.saturating_add(size).min(d.staging.len());
        if offset >= end {
            return Ok(());
        }
        let chunk = PLAINTEXT_CHUNK_LEN as u64;
        self.load_chunks(d, offset / chunk, (end - 1) / chunk).await
    }

    async fn load_all(&self, d: &DirtyFile) -> Result<(), OpError> {
        self.load_chunks(d, 0, u64::MAX).await
    }

    async fn drop_base_from(&self, d: &DirtyFile, from: u64) {
        let Some(lazy) = &d.lazy else {
            return;
        };
        let mut resolved = lazy.resolved.lock().await;
        let from = usize::try_from(from).unwrap_or(usize::MAX);
        for slot in resolved.iter_mut().skip(from) {
            *slot = true;
        }
        if resolved.iter().all(|r| *r) {
            lazy.pending.store(false, Ordering::Release);
        }
    }

    async fn prepare_resize(&self, d: &DirtyFile, size: u64) -> Result<(), OpError> {
        if d.lazy.is_none() || size >= d.staging.len() {
            return Ok(());
        }
        let chunk = PLAINTEXT_CHUNK_LEN as u64;
        let mut keep = size / chunk;
        if !size.is_multiple_of(chunk) {
            self.load_chunks(d, keep, keep).await?;
            keep += 1;
        }
        self.drop_base_from(d, keep).await;
        Ok(())
    }

    async fn commit(&self, ino: Ino, d: &Arc<DirtyFile>) -> Result<(), OpError> {
        let _guard = d.commit_lock.lock().await;
        if d.unlinked.load(Ordering::Acquire) || !d.modified.swap(false, Ordering::AcqRel) {
            if d.writers.load(Ordering::Acquire) == 0 {
                self.forget_dirty(ino);
            }
            return Ok(());
        }
        match self.publish(ino, d).await {
            Ok(()) => {
                d.existed.store(true, Ordering::Release);
                if d.writers.load(Ordering::Acquire) == 0 && !d.modified.load(Ordering::Acquire) {
                    self.forget_dirty(ino);
                }
                Ok(())
            }
            Err(e) => {
                d.modified.store(true, Ordering::Release);
                Err(e)
            }
        }
    }

    async fn publish(&self, ino: Ino, d: &Arc<DirtyFile>) -> Result<(), OpError> {
        let (parent, name) = self.file_location(ino)?;
        let dir = self.dir_ref(parent)?;
        let path = self.child_path(parent, &name)?;
        self.load_all(d).await?;
        d.staging.fsync()?;
        if d.existed.load(Ordering::Acquire) {
            match self.fs.kind_in(&self.config, &dir, &name).await {
                Ok(_) => self.trash_or_delete(&path).await?,
                Err(cu_vfs::VfsError::NotFound) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let mut source = d.staging.reader();
        self.fs
            .write_file_with_nonce_in(&self.config, &dir, &name, &mut source, None, &mut |_| {})
            .await?;
        Ok(())
    }

    async fn schedule_upload(self: &Arc<Self>, ino: Ino, d: &Arc<DirtyFile>) {
        let generation = d.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if self.load_all(d).await.is_ok() {
            let _ = self.write_journal(ino, d);
        }
        let (core, d) = (Arc::clone(self), Arc::clone(d));
        let delay = *self.upload_delay.lock().unwrap();
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let mut backoff = Duration::from_secs(2);
            for _ in 0..MAX_UPLOAD_ATTEMPTS {
                let superseded = d.generation.load(Ordering::Acquire) != generation
                    || d.writers.load(Ordering::Acquire) != 0
                    || d.unlinked.load(Ordering::Acquire);
                if superseded {
                    return;
                }
                if core.commit(ino, &d).await.is_ok() {
                    return;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        });
    }

    async fn commit_all(&self) -> usize {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline
            && self.all_dirty().iter().any(|(_, d)| {
                d.writers.load(Ordering::Acquire) > 0 && !d.unlinked.load(Ordering::Acquire)
            })
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut failed = 0;
        for (ino, d) in self.all_dirty() {
            if d.writers.load(Ordering::Acquire) == 0 && self.commit(ino, &d).await.is_err() {
                failed += 1;
            }
        }
        failed
    }

    async fn commit_under(&self, prefix: &str) -> Result<(), OpError> {
        let under = format!("{prefix}/");
        for (ino, d) in self.all_dirty() {
            let path = self.inodes.lock().unwrap().path_of(ino);
            if path.is_some_and(|p| p.starts_with(&under)) {
                self.commit(ino, &d).await?;
            }
        }
        Ok(())
    }

    async fn recover(&self) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.staging_dir) else {
            return 0;
        };
        let mut recovered = 0;
        for entry in entries.flatten() {
            let journal = entry.path();
            if journal.extension().and_then(|e| e.to_str()) != Some("jrn") {
                continue;
            }
            let Some(parsed) = std::fs::read(&journal)
                .ok()
                .and_then(|b| self.open_journal(&b))
            else {
                continue;
            };
            let stg = journal.with_extension("");
            let Ok(staging) = StagingFile::open(&stg, &self.config, parsed.0) else {
                continue;
            };
            if self
                .publish_recovered(Arc::new(staging), &parsed.2, parsed.1)
                .await
                .is_ok()
            {
                let _ = std::fs::remove_file(&stg);
                let _ = std::fs::remove_file(&journal);
                recovered += 1;
            }
        }
        recovered
    }

    async fn publish_recovered(
        &self,
        staging: Arc<StagingFile>,
        path: &str,
        existed: bool,
    ) -> Result<(), OpError> {
        let (parent_path, name) = split_path(path);
        let dir = self
            .fs
            .resolve_or_create_dir_ref(&self.config, parent_path)
            .await?;
        if existed {
            match self.fs.kind_in(&self.config, &dir, name).await {
                Ok(_) => self.trash_or_delete(path).await?,
                Err(cu_vfs::VfsError::NotFound) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let mut source = staging.reader();
        self.fs
            .write_file_with_nonce_in(&self.config, &dir, name, &mut source, None, &mut |_| {})
            .await?;
        Ok(())
    }
}

#[allow(clippy::unwrap_used)]
impl VfsOps {
    pub fn enable_writes(
        &mut self,
        staging_dir: PathBuf,
        hooks: Option<Arc<dyn MountWriteHooks>>,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(&staging_dir)?;
        self.write = Some(Arc::new(WriteCore {
            fs: Arc::clone(&self.fs),
            config: self.config.clone(),
            inodes: Arc::clone(&self.inodes),
            staging_dir,
            hooks,
            dirty: Mutex::new(HashMap::new()),
            materialize: tokio::sync::Mutex::new(()),
            counter: AtomicU64::new(0),
            upload_delay: Mutex::new(DEFAULT_UPLOAD_DELAY),
        }));
        Ok(())
    }

    #[must_use]
    pub fn is_writable(&self) -> bool {
        self.write.is_some()
    }

    fn core(&self) -> Result<&Arc<WriteCore>, OpError> {
        self.write.as_ref().ok_or(OpError::ReadOnly)
    }

    fn dirty_get(&self, ino: Ino) -> Option<Arc<DirtyFile>> {
        self.write.as_ref()?.dirty_get(ino)
    }

    pub(crate) fn dirty_attr(&self, ino: Ino) -> Option<Attr> {
        let d = self.dirty_get(ino)?;
        Some(Attr {
            kind: NodeKind::File,
            size: d.staging.len(),
        })
    }

    pub(crate) fn dirty_lookup(&self, parent: Ino, name: &str) -> Option<(Ino, Attr)> {
        self.write.as_ref()?;
        let ino = self.inodes.lock().unwrap().find(parent, name)?;
        let attr = self.dirty_attr(ino)?;
        Some((ino, attr))
    }

    pub(crate) fn read_dirty(
        &self,
        ino: Ino,
        offset: u64,
        size: u32,
    ) -> Option<Result<Vec<u8>, OpError>> {
        let d = self.dirty_get(ino)?;
        let mut buf = vec![0u8; usize::try_from(size).unwrap_or(0)];
        Some(match d.staging.read_at(offset, &mut buf) {
            Ok(n) => {
                buf.truncate(n);
                Ok(buf)
            }
            Err(e) => Err(e.into()),
        })
    }

    pub(crate) fn overlay_readdir(&self, dir: Ino, entries: &mut Vec<Entry>) {
        let Some(core) = self.write.as_ref() else {
            return;
        };
        for (ino, d) in core.all_dirty() {
            if d.unlinked.load(Ordering::Acquire) {
                continue;
            }
            let Ok((parent, name)) = core.file_location(ino) else {
                continue;
            };
            if parent != dir {
                continue;
            }
            let size = Some(d.staging.len());
            if let Some(e) = entries.iter_mut().find(|e| e.name == name) {
                e.size = size;
            } else {
                entries.push(Entry {
                    name,
                    kind: NodeKind::File,
                    size,
                    modified: None,
                });
            }
        }
    }

    pub async fn create(&self, parent: Ino, name: &str) -> Result<(Ino, Attr, Fh), OpError> {
        let core = self.core()?;
        core.child_path(parent, name)?;
        match self.lookup(parent, name).await {
            Ok(_) => return Err(OpError::Exists),
            Err(OpError::NotFound) => {}
            Err(e) => return Err(e),
        }
        let ino = self.inodes.lock().unwrap().register_file(parent, name);
        let staging_path = core.new_staging_path();
        let staging = Arc::new(StagingFile::create(&staging_path, &self.config)?);
        let dirty = Arc::new(DirtyFile {
            staging,
            lazy: None,
            journal: WriteCore::journal_path(&staging_path),
            path: staging_path,
            existed: AtomicBool::new(false),
            writers: AtomicUsize::new(1),
            modified: AtomicBool::new(true),
            unlinked: AtomicBool::new(false),
            journaled: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            commit_lock: tokio::sync::Mutex::new(()),
        });
        core.dirty.lock().unwrap().insert(ino, dirty);
        let fh = self.new_write_handle(ino);
        Ok((
            ino,
            Attr {
                kind: NodeKind::File,
                size: 0,
            },
            fh,
        ))
    }

    fn new_write_handle(&self, ino: Ino) -> Fh {
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
        self.handles.lock().unwrap().insert(
            fh,
            Arc::new(Handle {
                reader: None,
                ino,
                writable: true,
                readahead: Mutex::new(ReadaheadState::new()),
            }),
        );
        fh
    }

    async fn ensure_dirty(&self, ino: Ino, truncate: bool) -> Result<Arc<DirtyFile>, OpError> {
        let core = self.core()?;
        let _guard = core.materialize.lock().await;
        if let Some(d) = core.dirty_get(ino) {
            if truncate {
                core.drop_base_from(&d, 0).await;
                d.staging.set_len(0)?;
                d.modified.store(true, Ordering::Release);
            }
            return Ok(d);
        }
        if core.is_dir(ino) {
            return Err(OpError::IsADirectory);
        }
        let (parent, name) = core.file_location(ino)?;
        let dir = core.dir_ref(parent)?;
        let reader = self.fs.open_read_at_in(&self.config, &dir, &name).await?;
        let staging_path = core.new_staging_path();
        let staging = Arc::new(StagingFile::create(&staging_path, &self.config)?);
        let lazy = (!truncate).then(|| {
            let chunks = usize::try_from(reader.total_chunks()).unwrap_or(usize::MAX);
            LazyBase {
                pending: AtomicBool::new(chunks > 0),
                resolved: tokio::sync::Mutex::new(vec![false; chunks]),
                reader,
            }
        });
        if let Some(lazy) = &lazy {
            staging.set_len(lazy.reader.plaintext_len())?;
        }
        let d = Arc::new(DirtyFile {
            staging,
            lazy,
            journal: WriteCore::journal_path(&staging_path),
            path: staging_path,
            existed: AtomicBool::new(true),
            writers: AtomicUsize::new(0),
            modified: AtomicBool::new(truncate),
            unlinked: AtomicBool::new(false),
            journaled: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            commit_lock: tokio::sync::Mutex::new(()),
        });
        core.dirty.lock().unwrap().insert(ino, Arc::clone(&d));
        Ok(d)
    }

    fn location_or_dir_error(&self, ino: Ino) -> Result<(Ino, String), OpError> {
        let core = self.core()?;
        core.file_location(ino).map_err(|e| {
            if core.is_dir(ino) {
                OpError::IsADirectory
            } else {
                e
            }
        })
    }

    pub async fn open_write(&self, ino: Ino, truncate: bool) -> Result<Fh, OpError> {
        let core = self.core()?;
        let (parent, name) = self.location_or_dir_error(ino)?;
        core.child_path(parent, &name)?;
        let d = self.ensure_dirty(ino, truncate).await?;
        d.writers.fetch_add(1, Ordering::AcqRel);
        Ok(self.new_write_handle(ino))
    }

    pub fn write(&self, fh: Fh, offset: u64, data: &[u8]) -> Result<usize, OpError> {
        let handle = self
            .handles
            .lock()
            .unwrap()
            .get(&fh)
            .cloned()
            .ok_or(OpError::UnknownHandle)?;
        if !handle.writable {
            return Err(OpError::UnknownHandle);
        }
        let d = self.dirty_get(handle.ino).ok_or(OpError::UnknownHandle)?;
        if let Some(lazy) = d
            .lazy
            .as_ref()
            .filter(|l| l.pending.load(Ordering::Acquire))
        {
            let chunk = PLAINTEXT_CHUNK_LEN as u64;
            let loaded = lazy.resolved.try_lock().is_ok_and(|r| {
                let end = offset.saturating_add(data.len() as u64);
                (offset / chunk..=end.saturating_sub(1) / chunk).all(|i| {
                    usize::try_from(i)
                        .ok()
                        .and_then(|i| r.get(i))
                        .is_none_or(|x| *x)
                })
            });
            if !data.is_empty() && !loaded {
                return Err(OpError::Io("write to a chunk not downloaded yet".into()));
            }
        }
        d.staging.write_at(offset, data)?;
        d.modified.store(true, Ordering::Release);
        Ok(data.len())
    }

    #[must_use]
    pub fn write_needs_loading(&self, fh: Fh) -> bool {
        let Some(ino) = self.handles.lock().unwrap().get(&fh).map(|h| h.ino) else {
            return false;
        };
        self.dirty_get(ino).is_some_and(|d| {
            d.lazy
                .as_ref()
                .is_some_and(|l| l.pending.load(Ordering::Acquire))
        })
    }

    pub async fn write_loading(&self, fh: Fh, offset: u64, data: &[u8]) -> Result<usize, OpError> {
        let handle = self
            .handles
            .lock()
            .unwrap()
            .get(&fh)
            .cloned()
            .ok_or(OpError::UnknownHandle)?;
        if let (Some(core), Some(d)) = (self.write.as_ref(), self.dirty_get(handle.ino))
            && !data.is_empty()
        {
            let chunk = PLAINTEXT_CHUNK_LEN as u64;
            let end = offset.saturating_add(data.len() as u64);
            core.load_chunks(&d, offset / chunk, (end - 1) / chunk)
                .await?;
        }
        self.write(fh, offset, data)
    }

    pub(crate) async fn load_dirty_range(
        &self,
        ino: Ino,
        offset: u64,
        size: u32,
    ) -> Result<(), OpError> {
        if let (Some(core), Some(d)) = (self.write.as_ref(), self.dirty_get(ino)) {
            core.load_range(&d, offset, u64::from(size)).await?;
        }
        Ok(())
    }

    pub fn fsync(&self, fh: Fh) -> Result<(), OpError> {
        let handle = self.handles.lock().unwrap().get(&fh).cloned();
        if let Some(h) = handle.filter(|h| h.writable)
            && let Some(d) = self.dirty_get(h.ino)
        {
            d.staging.fsync()?;
        }
        Ok(())
    }

    pub async fn set_size(&self, ino: Ino, size: u64) -> Result<(), OpError> {
        let core = self.core()?;
        let (parent, name) = self.location_or_dir_error(ino)?;
        core.child_path(parent, &name)?;
        let d = self.ensure_dirty(ino, size == 0).await?;
        core.prepare_resize(&d, size).await?;
        d.staging.set_len(size)?;
        d.modified.store(true, Ordering::Release);
        if d.writers.load(Ordering::Acquire) == 0 {
            core.schedule_upload(ino, &d).await;
        }
        Ok(())
    }

    pub(crate) async fn release_writer(&self, ino: Ino) -> Result<(), OpError> {
        let core = self.core()?;
        let Some(d) = core.dirty_get(ino) else {
            return Ok(());
        };
        let previous = d.writers.fetch_sub(1, Ordering::AcqRel);
        if previous != 1 {
            return Ok(());
        }
        if d.unlinked.load(Ordering::Acquire) {
            core.forget_dirty(ino);
            return Ok(());
        }
        if d.modified.load(Ordering::Acquire) {
            core.schedule_upload(ino, &d).await;
        } else {
            core.forget_dirty(ino);
        }
        Ok(())
    }

    pub async fn commit_all_dirty(&self) -> usize {
        match self.write.as_ref() {
            Some(core) => core.commit_all().await,
            None => 0,
        }
    }

    pub async fn recover_pending(&self) -> usize {
        match self.write.as_ref() {
            Some(core) => core.recover().await,
            None => 0,
        }
    }

    #[must_use]
    pub fn pending_files(&self) -> usize {
        self.write.as_ref().map_or(0, |core| {
            core.dirty
                .lock()
                .unwrap()
                .values()
                .filter(|d| {
                    d.modified.load(Ordering::Acquire) && !d.unlinked.load(Ordering::Acquire)
                })
                .count()
        })
    }

    pub async fn mkdir(&self, parent: Ino, name: &str) -> Result<(Ino, Attr), OpError> {
        let core = self.core()?;
        let path = core.child_path(parent, name)?;
        match self.lookup(parent, name).await {
            Ok(_) => return Err(OpError::Exists),
            Err(OpError::NotFound) => {}
            Err(e) => return Err(e),
        }
        self.fs.mkdir(&self.config, &path).await?;
        self.lookup(parent, name).await
    }

    pub async fn rmdir(&self, parent: Ino, name: &str) -> Result<(), OpError> {
        let core = self.core()?;
        let path = core.child_path(parent, name)?;
        let (ino, attr) = self.lookup(parent, name).await?;
        if attr.kind != NodeKind::Dir {
            return Err(OpError::NotADirectory);
        }
        if !self.readdir(ino).await?.is_empty() {
            return Err(OpError::NotEmpty);
        }
        self.fs.remove_dir(&self.config, &path).await?;
        self.inodes.lock().unwrap().remove_entry(parent, name);
        Ok(())
    }

    pub async fn unlink(&self, parent: Ino, name: &str) -> Result<(), OpError> {
        let core = self.core()?;
        let path = core.child_path(parent, name)?;
        let (ino, attr) = self.lookup(parent, name).await?;
        if attr.kind == NodeKind::Dir {
            return Err(OpError::IsADirectory);
        }
        let in_vault = if let Some(d) = core.dirty_get(ino) {
            let _guard = d.commit_lock.lock().await;
            d.unlinked.store(true, Ordering::Release);
            let existed = d.existed.load(Ordering::Acquire);
            if d.writers.load(Ordering::Acquire) == 0 {
                core.forget_dirty(ino);
            }
            existed
        } else {
            true
        };
        if in_vault {
            core.trash_or_delete(&path).await?;
        }
        self.inodes.lock().unwrap().remove_entry(parent, name);
        Ok(())
    }

    pub async fn rename(
        &self,
        old_parent: Ino,
        old_name: &str,
        new_parent: Ino,
        new_name: &str,
        no_replace: bool,
    ) -> Result<(), OpError> {
        let core = self.core()?;
        let old_path = core.child_path(old_parent, old_name)?;
        let new_path = core.child_path(new_parent, new_name)?;
        let (src_ino, src_attr) = self.lookup(old_parent, old_name).await?;
        if old_path == new_path {
            return Ok(());
        }
        if new_path.starts_with(&format!("{old_path}/")) {
            return Err(OpError::Io(
                "a directory cannot be moved into itself".into(),
            ));
        }
        if src_attr.kind == NodeKind::Dir {
            core.commit_under(&old_path).await?;
        }
        let src_dirty = core.dirty_get(src_ino);
        let _src_guard = match &src_dirty {
            Some(d) => Some(d.commit_lock.lock().await),
            None => None,
        };
        match self.lookup(new_parent, new_name).await {
            Ok((dest_ino, dest_attr)) => {
                if no_replace {
                    return Err(OpError::Exists);
                }
                match (src_attr.kind, dest_attr.kind) {
                    (NodeKind::Dir, NodeKind::File) => return Err(OpError::NotADirectory),
                    (NodeKind::File, NodeKind::Dir) => return Err(OpError::IsADirectory),
                    (NodeKind::Dir, NodeKind::Dir) => {
                        if !self.readdir(dest_ino).await?.is_empty() {
                            return Err(OpError::NotEmpty);
                        }
                        self.fs.remove_dir(&self.config, &new_path).await?;
                    }
                    (NodeKind::File, NodeKind::File) => {
                        let in_vault = if let Some(d) = core.dirty_get(dest_ino) {
                            let _g = d.commit_lock.lock().await;
                            d.unlinked.store(true, Ordering::Release);
                            if d.writers.load(Ordering::Acquire) == 0 {
                                core.forget_dirty(dest_ino);
                            }
                            d.existed.load(Ordering::Acquire)
                        } else {
                            true
                        };
                        if in_vault {
                            core.trash_or_delete(&new_path).await?;
                        }
                    }
                }
            }
            Err(OpError::NotFound) => {}
            Err(e) => return Err(e),
        }
        if let Some(d) = &src_dirty {
            core.load_all(d).await?;
        }
        let only_in_overlay = src_attr.kind == NodeKind::File
            && src_dirty
                .as_ref()
                .is_some_and(|d| !d.existed.load(Ordering::Acquire));
        if !only_in_overlay {
            self.fs.rename(&self.config, &old_path, &new_path).await?;
        }
        self.inodes
            .lock()
            .unwrap()
            .rename_entry(old_parent, old_name, new_parent, new_name);
        core.refresh_journal(src_ino);
        Ok(())
    }
}
