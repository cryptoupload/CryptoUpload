// SPDX-License-Identifier: GPL-3.0-or-later

pub mod error;
mod observer;

pub use error::VfsError;
pub use observer::NamespaceObserver;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use cu_backend::{BackendError, EntryKind as BackendEntryKind, ObjPath, StorageBackend};
use cu_crypto::Secret;
use cu_crypto::aead::{ContentKey, NONCE_LEN};
use cu_format::common::SeedId;
use cu_format::config::{FormatProfile, VaultConfig};
use cu_format::content::{
    CIPHERTEXT_CHUNK_LEN, ContentDecryptor, ContentEncryptor, MIN_CIPHERTEXT_CHUNK,
    PLAINTEXT_CHUNK_LEN,
};
use cu_format::error::FormatError;
use cu_format::header::HEADER_LEN;
use cu_format::manifest_v2::{EntryKey, Node, Op};
use cu_format::{dirid, dirmeta, header, keys, name, padding};

mod decoys;
use futures_util::StreamExt as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const DIR_LINK_NAME: &str = "dir.uvf";
pub const DIRMETA_NAME: &str = "dirmeta.uvf";

const LIST_DETAILED_CONCURRENCY: usize = 8;

const SOURCE_READ_CHUNKS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
}

pub type HeaderNonce = [u8; NONCE_LEN];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: NodeKind,
    pub size: Option<u64>,
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickSize {
    Bytes(u64),
    MalformedGeometry,
}

fn is_padded(config: &VaultConfig) -> bool {
    config.profile() == FormatProfile::Private
}

#[must_use]
pub fn expected_quick_size(config: &VaultConfig, content_len: u64) -> Option<u64> {
    if is_padded(config) {
        padding::padded_stream_len(content_len)
    } else {
        Some(content_len)
    }
}

fn padded_bytes(plaintext: &[u8]) -> Result<Vec<u8>, VfsError> {
    let len = plaintext.len() as u64;
    let total = padding::padded_stream_len(len)
        .and_then(|total| usize::try_from(total).ok())
        .ok_or(FormatError::Malformed("object too large"))?;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(plaintext);
    out.resize(total - 8, 0);
    out.extend_from_slice(&len.to_be_bytes());
    Ok(out)
}

fn strip_padding(mut stream: Vec<u8>) -> Result<Vec<u8>, VfsError> {
    let tail_start = stream.len() - stream.len() % PLAINTEXT_CHUNK_LEN;
    let len = padding::content_len(stream.len() as u64, &stream[tail_start..])?;
    stream.truncate(usize::try_from(len).map_err(|_| FormatError::DecryptFailed)?);
    Ok(stream)
}

fn plaintext_len(ciphertext_len: u64) -> QuickSize {
    let Some(body_len) = ciphertext_len.checked_sub(HEADER_LEN as u64) else {
        return QuickSize::MalformedGeometry;
    };
    let full_chunks = body_len / CIPHERTEXT_CHUNK_LEN as u64;
    let remainder = body_len % CIPHERTEXT_CHUNK_LEN as u64;
    if remainder < MIN_CIPHERTEXT_CHUNK as u64 {
        return QuickSize::MalformedGeometry;
    }
    QuickSize::Bytes(
        full_chunks * PLAINTEXT_CHUNK_LEN as u64 + (remainder - MIN_CIPHERTEXT_CHUNK as u64),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailedEntry {
    pub name: String,
    pub encrypted_name: String,
    pub kind: NodeKind,
    pub dir_id: Option<[u8; 32]>,
}

#[derive(Clone)]
pub struct DirRef(DirHandle);

impl DirRef {
    #[must_use]
    pub fn dir_id(&self) -> [u8; 32] {
        self.0.dir_id
    }

    #[must_use]
    pub fn same_dir(&self, other: &Self) -> bool {
        self.0.dir_id == other.0.dir_id
    }
}

#[derive(Debug)]
pub struct FileReader {
    physical: ObjPath,
    content_key: ContentKey,
    header_nonce: [u8; NONCE_LEN],
    plaintext_len: u64,
    tail: (u64, Vec<u8>),
    cache_path: Arc<str>,
}

impl FileReader {
    #[must_use]
    pub const fn plaintext_len(&self) -> u64 {
        self.plaintext_len
    }

    #[must_use]
    pub const fn total_chunks(&self) -> u64 {
        self.plaintext_len / PLAINTEXT_CHUNK_LEN as u64 + 1
    }

    fn content_bytes_in_chunk(&self, index: u64) -> usize {
        let start = index.saturating_mul(PLAINTEXT_CHUNK_LEN as u64);
        usize::try_from(self.plaintext_len.saturating_sub(start))
            .unwrap_or(usize::MAX)
            .min(PLAINTEXT_CHUNK_LEN)
    }

    #[must_use]
    pub fn tail(&self) -> Option<(u64, &[u8])> {
        let last = self.total_chunks() - 1;
        (last == self.tail.0).then(|| {
            let keep = self.content_bytes_in_chunk(last).min(self.tail.1.len());
            (last, &self.tail.1[..keep])
        })
    }

    #[must_use]
    pub fn cache_key(&self) -> (Arc<str>, [u8; NONCE_LEN]) {
        (Arc::clone(&self.cache_path), self.header_nonce)
    }
}

#[derive(Clone)]
struct DirHandle {
    dir_id: [u8; 32],
    seed: Secret<32>,
    physical: ObjPath,
}

struct DirScan {
    entries: Vec<DetailedEntry>,
    decoys: Vec<DetailedEntry>,
    real_files: Vec<RealFile>,
}

struct RealFile {
    name_len: usize,
    stored_len: Option<u64>,
}

type HeaderKeyFn = fn(&Secret<32>, &[u8; 32]) -> Result<Secret<32>, FormatError>;

pub struct VaultFs {
    backend: Box<dyn StorageBackend>,
    pending_dirmeta: Mutex<Option<HashMap<String, DirHandle>>>,
    observer: Option<Arc<dyn NamespaceObserver>>,
    untracked_dirs: Mutex<HashSet<[u8; 32]>>,
    internal_dirs: Mutex<HashMap<String, DirHandle>>,
}

impl VaultFs {
    #[must_use]
    pub fn new(backend: Box<dyn StorageBackend>) -> Self {
        Self {
            backend,
            pending_dirmeta: Mutex::new(None),
            observer: None,
            untracked_dirs: Mutex::new(HashSet::new()),
            internal_dirs: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn NamespaceObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    pub fn set_observer(&mut self, observer: Arc<dyn NamespaceObserver>) {
        self.observer = Some(observer);
    }

    fn is_untracked(&self, dir_id: &[u8; 32]) -> bool {
        self.untracked_dirs
            .lock()
            .expect("mutex poisoned")
            .contains(dir_id)
    }

    pub fn clear_internal_dir_cache(&self) {
        self.internal_dirs.lock().expect("mutex poisoned").clear();
    }

    fn observes(&self, dir_id: &[u8; 32]) -> bool {
        self.observer.is_some()
            && !self
                .untracked_dirs
                .lock()
                .expect("mutex poisoned")
                .contains(dir_id)
    }

    async fn observe_begin(&self, ops: Vec<Op>) -> Result<Option<Vec<Op>>, VfsError> {
        let Some(observer) = &self.observer else {
            return Ok(None);
        };
        if ops.is_empty() {
            return Ok(None);
        }
        observer.begin(&ops).await?;
        Ok(Some(ops))
    }

    fn guard_key(&self, parent: &DirHandle, entry_name: &str) -> Option<EntryKey> {
        self.observes(&parent.dir_id)
            .then(|| Self::entry_key(parent, entry_name))
    }

    async fn guard_file(
        &self,
        key: Option<&EntryKey>,
        served: &[u8; NONCE_LEN],
    ) -> Result<(), VfsError> {
        match (&self.observer, key) {
            (Some(observer), Some(key)) => observer.check_file(key, served).await,
            _ => Ok(()),
        }
    }

    async fn announce_delete(
        &self,
        parent: &DirHandle,
        entry_name: &str,
    ) -> Result<Option<Vec<Op>>, VfsError> {
        if !self.observes(&parent.dir_id) {
            return Ok(None);
        }
        self.observe_begin(vec![Op::Del {
            key: Self::entry_key(parent, entry_name),
        }])
        .await
    }

    async fn announce_rename(
        &self,
        config: &VaultConfig,
        src: (&DirHandle, &str),
        dst: (&DirHandle, &str),
        src_physical: &ObjPath,
        kind: NodeKind,
    ) -> Result<Option<Vec<Op>>, VfsError> {
        let src_key = Self::entry_key(src.0, src.1);
        let dst_key = Self::entry_key(dst.0, dst.1);
        if self.observer.is_none() || src_key == dst_key {
            return Ok(None);
        }
        let track_dst = self.observes(&dst.0.dir_id);
        let track_src = self.observes(&src.0.dir_id);
        if !track_dst && !track_src {
            return Ok(None);
        }
        let node = match kind {
            NodeKind::File => {
                let salt = *config.kdf_salt();
                let header_bytes = self
                    .backend
                    .get_range(src_physical, 0, HEADER_LEN as u64)
                    .await?;
                let (_key, nonce, _seed_id) = header::open(
                    &header_bytes,
                    |id| config.seed(id).cloned(),
                    |seed| keys::header_key(seed, &salt),
                )?;
                Node::File { nonce }
            }
            NodeKind::Dir => {
                let (dir_id, _seed_id) = self
                    .read_dir_uvf(config, &src_physical.join(DIR_LINK_NAME)?)
                    .await?;
                Node::Dir { dir_id }
            }
        };
        let mut ops = Vec::with_capacity(2);
        if track_dst {
            ops.push(Op::Put { key: dst_key, node });
        }
        if track_src {
            ops.push(Op::Del { key: src_key });
        }
        self.observe_begin(ops).await
    }

    async fn observe_end(&self, announced: Option<Vec<Op>>, committed: bool) {
        if let (Some(observer), Some(ops)) = (&self.observer, announced) {
            observer.end(&ops, committed).await;
        }
    }

    fn entry_key(parent: &DirHandle, entry_name: &str) -> EntryKey {
        EntryKey {
            parent: parent.dir_id,
            name: name::nfc(entry_name).into_owned(),
        }
    }

    pub fn begin_dirmeta_batch(&self) {
        let mut guard = self.pending_dirmeta.lock().expect("mutex poisoned");
        *guard = Some(HashMap::new());
    }

    pub async fn end_dirmeta_batch(&self, config: &VaultConfig) {
        let pending = {
            let mut guard = self.pending_dirmeta.lock().expect("mutex poisoned");
            guard.take()
        };
        let Some(pending) = pending else { return };
        for dir in pending.into_values() {
            self.refresh_dirmeta_now(config, &dir).await;
        }
    }

    #[must_use]
    pub fn backend(&self) -> &dyn StorageBackend {
        self.backend.as_ref()
    }

    pub async fn init_root(&self, config: &VaultConfig) -> Result<(), VfsError> {
        let salt = *config.kdf_salt();
        let root_seed_id = config.initial_seed_id();
        let root_seed = config.initial_seed().clone();
        let root_dir_id = dirid::root_dir_id(&root_seed, &salt)?;
        let root_physical = ObjPath::new(dirid::physical_path(&root_dir_id, &root_seed, &salt)?)?;
        self.backend.create_dir_all(&root_physical).await?;
        self.write_node_bytes(
            config,
            &root_physical.join(DIR_LINK_NAME)?,
            root_seed_id,
            &root_seed,
            &root_dir_id,
            keys::header_key,
            false,
            false,
        )
        .await?;
        Ok(())
    }

    pub async fn list(
        &self,
        config: &VaultConfig,
        vault_dir: &str,
    ) -> Result<Vec<Entry>, VfsError> {
        let segments = split_logical_path(vault_dir)?;
        let dir = self.resolve_dir(config, &segments).await?;
        self.list_handle(config, &dir).await
    }

    async fn list_handle(
        &self,
        config: &VaultConfig,
        dir: &DirHandle,
    ) -> Result<Vec<Entry>, VfsError> {
        let raw = self.backend.list(&dir.physical).await?;
        let cache = self
            .try_read_dirmeta_cache(config, dir)
            .await
            .unwrap_or_default();
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&dir.seed, &salt)?;

        let mut out = Vec::with_capacity(raw.len());
        for item in raw {
            if item.name == DIR_LINK_NAME || item.name == DIRMETA_NAME {
                continue;
            }
            let Some(encrypted_name) = item.name.strip_suffix(name::SUFFIX) else {
                continue;
            };
            let kind = match item.kind {
                BackendEntryKind::File => NodeKind::File,
                BackendEntryKind::Directory => NodeKind::Dir,
            };
            let cleartext = match cache.get(encrypted_name) {
                Some(cached) if name::is_single_component(&cached.name) => cached.name.clone(),
                _ => name::decrypt_in(config.profile(), &siv_key, &dir.dir_id, &item.name)?,
            };
            if name::is_decoy(&cleartext) {
                continue;
            }
            let size = match (kind, item.size) {
                (NodeKind::File, Some(len)) if !is_padded(config) => match plaintext_len(len) {
                    QuickSize::Bytes(n) => Some(n),
                    QuickSize::MalformedGeometry => None,
                },
                _ => None,
            };
            out.push(Entry {
                name: cleartext,
                kind,
                size,
                modified: item.modified,
            });
        }
        Ok(out)
    }

    pub async fn list_detailed(
        &self,
        config: &VaultConfig,
        vault_dir: &str,
    ) -> Result<Vec<DetailedEntry>, VfsError> {
        let segments = split_logical_path(vault_dir)?;
        let dir = self.resolve_dir(config, &segments).await?;
        self.list_detailed_handle(config, &dir).await
    }

    pub async fn physical_dir(
        &self,
        config: &VaultConfig,
        vault_dir: &str,
    ) -> Result<String, VfsError> {
        let segments = split_logical_path(vault_dir)?;
        let dir = self.resolve_dir(config, &segments).await?;
        Ok(dir.physical.as_str().to_string())
    }

    async fn list_detailed_handle(
        &self,
        config: &VaultConfig,
        dir: &DirHandle,
    ) -> Result<Vec<DetailedEntry>, VfsError> {
        Ok(self.scan_dir(config, dir).await?.entries)
    }

    async fn scan_dir(&self, config: &VaultConfig, dir: &DirHandle) -> Result<DirScan, VfsError> {
        let mut decoys = Vec::new();
        let mut real_files = Vec::new();
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&dir.seed, &salt)?;
        let raw = self.backend.list(&dir.physical).await?;
        let mut out = Vec::with_capacity(raw.len());
        let mut pending_dirs: Vec<(usize, ObjPath)> = Vec::new();
        for item in raw {
            if item.name == DIR_LINK_NAME || item.name == DIRMETA_NAME {
                continue;
            }
            let Some(encrypted_name) = item.name.strip_suffix(name::SUFFIX) else {
                continue;
            };
            let encrypted_name = encrypted_name.to_string();
            let cleartext = name::decrypt_in(config.profile(), &siv_key, &dir.dir_id, &item.name)?;
            if name::is_decoy(&cleartext) {
                decoys.push(DetailedEntry {
                    name: cleartext,
                    encrypted_name,
                    kind: NodeKind::File,
                    dir_id: None,
                });
                continue;
            }
            if item.kind == BackendEntryKind::File {
                real_files.push(RealFile {
                    name_len: cleartext.len(),
                    stored_len: item.size,
                });
            }
            let kind = match item.kind {
                BackendEntryKind::File => NodeKind::File,
                BackendEntryKind::Directory => {
                    let link_path = dir.physical.join(&item.name)?.join(DIR_LINK_NAME)?;
                    pending_dirs.push((out.len(), link_path));
                    NodeKind::Dir
                }
            };
            out.push(DetailedEntry {
                name: cleartext,
                encrypted_name,
                kind,
                dir_id: None,
            });
        }
        let resolved =
            futures_util::stream::iter(pending_dirs)
                .map(|(idx, link_path)| async move {
                    (idx, self.read_dir_uvf(config, &link_path).await)
                })
                .buffered(LIST_DETAILED_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;
        for (idx, result) in resolved {
            let (child_dir_id, _seed_id) = result?;
            if let Some(entry) = out.get_mut(idx) {
                entry.dir_id = Some(child_dir_id);
            }
        }
        Ok(DirScan {
            entries: out,
            decoys,
            real_files,
        })
    }

    async fn rebalance_decoys(&self, config: &VaultConfig, dir: &DirHandle, scan: &mut DirScan) {
        let wanted = decoys::wanted(scan.real_files.len());
        while scan.decoys.len() > wanted {
            let Some(victim) = decoys::pick(scan.decoys.len()) else {
                return;
            };
            let entry = scan.decoys.swap_remove(victim);
            let Ok(path) = dir
                .physical
                .join(&format!("{}{}", entry.encrypted_name, name::SUFFIX))
            else {
                return;
            };
            if self.backend.delete(&path).await.is_err() {
                return;
            }
        }
        while scan.decoys.len() < wanted {
            let Ok(entry) = self.write_decoy(config, dir, &scan.real_files).await else {
                return;
            };
            scan.decoys.push(entry);
        }
    }

    async fn write_decoy(
        &self,
        config: &VaultConfig,
        dir: &DirHandle,
        real_files: &[RealFile],
    ) -> Result<DetailedEntry, VfsError> {
        let (decoy_name, content_len) = decoys::shape(real_files)?;
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&dir.seed, &salt)?;
        let encrypted = name::encrypt_decoy(&siv_key, &dir.dir_id, &decoy_name)?;
        let dest = dir.physical.join(&encrypted)?;
        let seed = config.latest_seed().clone();
        let mut zeros = tokio::io::repeat(0).take(content_len);
        self.seal_stream(
            config,
            &dest,
            config.latest_seed_id(),
            &seed,
            &mut zeros,
            None,
            &mut |_| {},
            false,
            None,
        )
        .await?;
        Ok(DetailedEntry {
            name: decoy_name,
            encrypted_name: encrypted
                .strip_suffix(name::SUFFIX)
                .unwrap_or(&encrypted)
                .to_string(),
            kind: NodeKind::File,
            dir_id: None,
        })
    }

    pub async fn mkdir(&self, config: &VaultConfig, vault_path: &str) -> Result<(), VfsError> {
        let (parent_segments, name) = split_parent_and_name(vault_path)?;
        let parent = self
            .resolve_or_create_dir(config, &parent_segments, true)
            .await?;
        self.create_child_dir(config, &parent, name, true).await?;
        self.refresh_dirmeta(config, &parent).await;
        Ok(())
    }

    pub async fn write_file(
        &self,
        config: &VaultConfig,
        vault_dest: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), VfsError> {
        self.write_file_mode(config, vault_dest, source, content_hash, on_bytes, false)
            .await
            .map(|_| ())
    }

    pub async fn write_file_with_nonce(
        &self,
        config: &VaultConfig,
        vault_dest: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<HeaderNonce, VfsError> {
        self.write_file_mode(config, vault_dest, source, content_hash, on_bytes, false)
            .await
    }

    pub async fn write_file_if_absent(
        &self,
        config: &VaultConfig,
        vault_dest: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), VfsError> {
        self.write_file_mode(config, vault_dest, source, content_hash, on_bytes, true)
            .await
            .map(|_| ())
    }

    pub async fn write_file_if_absent_with_nonce(
        &self,
        config: &VaultConfig,
        vault_dest: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<HeaderNonce, VfsError> {
        self.write_file_mode(config, vault_dest, source, content_hash, on_bytes, true)
            .await
    }

    async fn write_file_mode(
        &self,
        config: &VaultConfig,
        vault_dest: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
        exclusive: bool,
    ) -> Result<HeaderNonce, VfsError> {
        let (parent_segments, file_name) = split_parent_and_name(vault_dest)?;
        let parent = self
            .resolve_or_create_dir(config, &parent_segments, true)
            .await?;
        self.write_file_in_handle(
            config,
            &parent,
            file_name,
            source,
            content_hash,
            on_bytes,
            exclusive,
        )
        .await
    }

    pub async fn write_file_if_absent_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), VfsError> {
        self.write_file_in_handle(
            config,
            &dir.0,
            file_name,
            source,
            content_hash,
            on_bytes,
            true,
        )
        .await
        .map(|_| ())
    }

    pub async fn write_file_if_absent_with_nonce_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<HeaderNonce, VfsError> {
        self.write_file_in_handle(
            config,
            &dir.0,
            file_name,
            source,
            content_hash,
            on_bytes,
            true,
        )
        .await
    }

    pub async fn write_file_with_nonce_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<HeaderNonce, VfsError> {
        self.write_file_in_handle(
            config,
            &dir.0,
            file_name,
            source,
            content_hash,
            on_bytes,
            false,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_file_in_handle(
        &self,
        config: &VaultConfig,
        parent: &DirHandle,
        file_name: &str,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
        exclusive: bool,
    ) -> Result<HeaderNonce, VfsError> {
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&parent.seed, &salt)?;
        let encrypted = name::encrypt_in(config.profile(), &siv_key, &parent.dir_id, file_name)?;
        let dest = parent.physical.join(&encrypted)?;
        let seed_id = config.latest_seed_id();
        let seed = config.latest_seed().clone();
        let track = self
            .observes(&parent.dir_id)
            .then(|| Self::entry_key(parent, file_name));
        let nonce = self
            .seal_stream(
                config,
                &dest,
                seed_id,
                &seed,
                source,
                content_hash,
                on_bytes,
                exclusive,
                track,
            )
            .await?;
        if !self.is_untracked(&parent.dir_id) {
            self.refresh_dirmeta(config, parent).await;
        }
        Ok(nonce)
    }

    pub async fn read_file(
        &self,
        config: &VaultConfig,
        vault_src: &str,
        sink: &mut (dyn AsyncWrite + Unpin + Send),
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), VfsError> {
        self.read_file_with_nonce(config, vault_src, sink, on_bytes)
            .await
            .map(|_| ())
    }

    pub async fn read_file_with_nonce(
        &self,
        config: &VaultConfig,
        vault_src: &str,
        sink: &mut (dyn AsyncWrite + Unpin + Send),
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<HeaderNonce, VfsError> {
        let (parent_segments, file_name) = split_parent_and_name(vault_src)?;
        let parent = self.resolve_dir(config, &parent_segments).await?;
        let src = Self::resolve_file_physical(config, &parent, file_name)?;
        let guard = self.guard_key(&parent, file_name);
        self.open_stream(config, &src, sink, on_bytes, guard).await
    }

    pub async fn read_header_nonce(
        &self,
        config: &VaultConfig,
        vault_path: &str,
    ) -> Result<HeaderNonce, VfsError> {
        let (parent_segments, file_name) = split_parent_and_name(vault_path)?;
        let parent = self.resolve_dir(config, &parent_segments).await?;
        let src = Self::resolve_file_physical(config, &parent, file_name)?;
        let salt = *config.kdf_salt();
        let header_bytes = self.backend.get_range(&src, 0, HEADER_LEN as u64).await?;
        let (_content_key, header_nonce, _seed_id) = header::open(
            &header_bytes,
            |id| config.seed(id).cloned(),
            |seed| keys::header_key(seed, &salt),
        )?;
        Ok(header_nonce)
    }

    pub async fn resolve_dir_ref(
        &self,
        config: &VaultConfig,
        vault_dir: &str,
    ) -> Result<DirRef, VfsError> {
        let segments = split_logical_path(vault_dir)?;
        Ok(DirRef(self.resolve_dir(config, &segments).await?))
    }

    pub async fn resolve_or_create_dir_ref(
        &self,
        config: &VaultConfig,
        vault_dir: &str,
    ) -> Result<DirRef, VfsError> {
        let segments = split_logical_path(vault_dir)?;
        Ok(DirRef(
            self.resolve_or_create_dir(config, &segments, true).await?,
        ))
    }

    pub async fn resolve_dir_ref_untracked(
        &self,
        config: &VaultConfig,
        vault_dir: &str,
    ) -> Result<DirRef, VfsError> {
        if let Some(cached) = self.cached_internal_dir(vault_dir) {
            return Ok(DirRef(cached));
        }
        let segments = split_logical_path(vault_dir)?;
        let handle = self.resolve_dir_guarded(config, &segments, false).await?;
        self.remember_internal_dir(vault_dir, &handle);
        Ok(DirRef(handle))
    }

    fn cached_internal_dir(&self, vault_dir: &str) -> Option<DirHandle> {
        self.internal_dirs
            .lock()
            .expect("mutex poisoned")
            .get(vault_dir)
            .cloned()
    }

    fn remember_internal_dir(&self, vault_dir: &str, handle: &DirHandle) {
        self.internal_dirs
            .lock()
            .expect("mutex poisoned")
            .insert(vault_dir.to_string(), handle.clone());
    }

    pub async fn read_internal_file(
        &self,
        config: &VaultConfig,
        vault_src: &str,
        sink: &mut (dyn AsyncWrite + Unpin + Send),
    ) -> Result<(), VfsError> {
        let (parent_segments, file_name) = split_parent_and_name(vault_src)?;
        let parent = self
            .resolve_dir_guarded(config, &parent_segments, false)
            .await?;
        let src = Self::resolve_file_physical(config, &parent, file_name)?;
        self.open_stream(config, &src, sink, &mut |_| {}, None)
            .await
            .map(|_| ())
    }

    pub async fn read_header_nonce_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
    ) -> Result<HeaderNonce, VfsError> {
        let src = Self::resolve_file_physical(config, &dir.0, file_name)?;
        let salt = *config.kdf_salt();
        let header_bytes = self.backend.get_range(&src, 0, HEADER_LEN as u64).await?;
        let (_content_key, header_nonce, _seed_id) = header::open(
            &header_bytes,
            |id| config.seed(id).cloned(),
            |seed| keys::header_key(seed, &salt),
        )?;
        Ok(header_nonce)
    }

    pub async fn descend_dir_ref_untracked(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        child_name: &str,
    ) -> Result<DirRef, VfsError> {
        Ok(DirRef(
            self.descend(config, &dir.0, child_name, false).await?,
        ))
    }

    pub async fn read_internal_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
        sink: &mut (dyn AsyncWrite + Unpin + Send),
    ) -> Result<(), VfsError> {
        let src = Self::resolve_file_physical(config, &dir.0, file_name)?;
        self.open_stream(config, &src, sink, &mut |_| {}, None)
            .await
            .map(|_| ())
    }

    pub async fn resolve_or_create_dir_ref_untracked(
        &self,
        config: &VaultConfig,
        vault_dir: &str,
    ) -> Result<DirRef, VfsError> {
        let handle = if let Some(cached) = self.cached_internal_dir(vault_dir) {
            cached
        } else {
            let segments = split_logical_path(vault_dir)?;
            let handle = self.resolve_or_create_dir(config, &segments, false).await?;
            self.remember_internal_dir(vault_dir, &handle);
            handle
        };
        self.untracked_dirs
            .lock()
            .expect("mutex poisoned")
            .insert(handle.dir_id);
        Ok(DirRef(handle))
    }

    pub async fn descend_dir_ref(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        child_name: &str,
    ) -> Result<DirRef, VfsError> {
        Ok(DirRef(
            self.descend(config, &dir.0, child_name, true).await?,
        ))
    }

    pub async fn list_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
    ) -> Result<Vec<Entry>, VfsError> {
        self.list_handle(config, &dir.0).await
    }

    pub async fn read_file_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
        sink: &mut (dyn AsyncWrite + Unpin + Send),
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), VfsError> {
        let src = Self::resolve_file_physical(config, &dir.0, file_name)?;
        let guard = self.guard_key(&dir.0, file_name);
        self.open_stream(config, &src, sink, on_bytes, guard)
            .await
            .map(|_| ())
    }

    pub async fn remove_file_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
    ) -> Result<(), VfsError> {
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&dir.0.seed, &salt)?;
        let encrypted = name::encrypt_in(config.profile(), &siv_key, &dir.0.dir_id, file_name)?;
        let target = dir.0.physical.join(&encrypted)?;
        if !self.backend.exists(&target).await? {
            return Err(VfsError::NotFound);
        }
        let announced = self.announce_delete(&dir.0, file_name).await?;
        let deleted = self.backend.delete(&target).await;
        self.observe_end(announced, deleted.is_ok()).await;
        deleted?;
        if !self.is_untracked(&dir.0.dir_id) {
            self.refresh_dirmeta(config, &dir.0).await;
        }
        Ok(())
    }

    pub async fn open_read_at(
        &self,
        config: &VaultConfig,
        vault_path: &str,
    ) -> Result<FileReader, VfsError> {
        let (parent_segments, file_name) = split_parent_and_name(vault_path)?;
        let parent = self.resolve_dir(config, &parent_segments).await?;
        let physical = Self::resolve_file_physical(config, &parent, file_name)?;
        let guard = self.guard_key(&parent, file_name);
        self.open_reader_at_physical(config, physical, guard).await
    }

    pub async fn open_read_at_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
    ) -> Result<FileReader, VfsError> {
        let physical = Self::resolve_file_physical(config, &dir.0, file_name)?;
        let guard = self.guard_key(&dir.0, file_name);
        self.open_reader_at_physical(config, physical, guard).await
    }

    pub async fn read_chunk(&self, reader: &FileReader, index: u64) -> Result<Vec<u8>, VfsError> {
        Ok(self
            .read_chunks(reader, index, 1)
            .await?
            .pop()
            .unwrap_or_default())
    }

    pub async fn read_chunks(
        &self,
        reader: &FileReader,
        first: u64,
        count: u64,
    ) -> Result<Vec<Vec<u8>>, VfsError> {
        let tail_index = reader.tail.0;
        let end = first.saturating_add(count).min(reader.total_chunks());
        if first >= end {
            return Ok(Vec::new());
        }
        let fetched_end = end.min(tail_index);
        let mut out = Vec::with_capacity(usize::try_from(end - first).unwrap_or(0));
        if first < fetched_end {
            let ct_len = CIPHERTEXT_CHUNK_LEN as u64;
            let offset = HEADER_LEN as u64 + first * ct_len;
            let ciphertext = self
                .backend
                .get_range(&reader.physical, offset, (fetched_end - first) * ct_len)
                .await?;
            let expected = usize::try_from((fetched_end - first) * ct_len)
                .map_err(|_| FormatError::DecryptFailed)?;
            if ciphertext.len() != expected {
                return Err(FormatError::DecryptFailed.into());
            }
            for (index, chunk) in (first..fetched_end).zip(ciphertext.chunks(CIPHERTEXT_CHUNK_LEN))
            {
                out.push(
                    cu_crypto::aead::open_chunk(
                        &reader.content_key,
                        index,
                        &reader.header_nonce,
                        chunk,
                    )
                    .map_err(FormatError::from)?,
                );
            }
        }
        if end > tail_index {
            out.push(reader.tail.1.clone());
        }
        for (index, chunk) in (first..end).zip(out.iter_mut()) {
            chunk.truncate(reader.content_bytes_in_chunk(index));
        }
        Ok(out)
    }

    async fn open_reader_at_physical(
        &self,
        config: &VaultConfig,
        physical: ObjPath,
        guard: Option<EntryKey>,
    ) -> Result<FileReader, VfsError> {
        let salt = *config.kdf_salt();
        let stat = self.backend.stat(&physical).await?;
        let geometry = Self::terminal_geometry(stat.size);
        let header_read = self.backend.get_range(&physical, 0, HEADER_LEN as u64);
        let (header_bytes, terminal_read) = match &geometry {
            Ok((full_chunks, _)) => {
                let terminal_offset = HEADER_LEN as u64 + full_chunks * CIPHERTEXT_CHUNK_LEN as u64;
                let terminal_read =
                    self.backend
                        .get_range(&physical, terminal_offset, CIPHERTEXT_CHUNK_LEN as u64);
                let (header, terminal) =
                    futures_util::future::join(header_read, terminal_read).await;
                (header?, Some(terminal))
            }
            Err(_) => (header_read.await?, None),
        };
        let (content_key, header_nonce, _seed_id) = header::open(
            &header_bytes,
            |id| config.seed(id).cloned(),
            |seed| keys::header_key(seed, &salt),
        )?;
        self.guard_file(guard.as_ref(), &header_nonce).await?;

        let (full_chunks, remainder) = geometry?;
        let terminal_ciphertext = terminal_read.expect("read when the geometry is valid")?;
        let terminal_plaintext = cu_crypto::aead::open_chunk(
            &content_key,
            full_chunks,
            &header_nonce,
            &terminal_ciphertext,
        )
        .map_err(FormatError::from)?;
        if terminal_plaintext.len() as u64 != remainder - MIN_CIPHERTEXT_CHUNK as u64 {
            return Err(VfsError::Format(FormatError::DecryptFailed));
        }

        let stream_len = full_chunks * PLAINTEXT_CHUNK_LEN as u64 + terminal_plaintext.len() as u64;
        let plaintext_len = if is_padded(config) {
            padding::content_len(stream_len, &terminal_plaintext)?
        } else {
            stream_len
        };

        let cache_path: Arc<str> = Arc::from(physical.as_str());

        Ok(FileReader {
            physical,
            content_key,
            header_nonce,
            plaintext_len,
            tail: (full_chunks, terminal_plaintext),
            cache_path,
        })
    }

    fn terminal_geometry(size: u64) -> Result<(u64, u64), VfsError> {
        let body_len = size
            .checked_sub(HEADER_LEN as u64)
            .ok_or(FormatError::DecryptFailed)?;
        let remainder = body_len % CIPHERTEXT_CHUNK_LEN as u64;
        if remainder < MIN_CIPHERTEXT_CHUNK as u64 {
            return Err(VfsError::Format(FormatError::DecryptFailed));
        }
        Ok((body_len / CIPHERTEXT_CHUNK_LEN as u64, remainder))
    }

    fn resolve_file_physical(
        config: &VaultConfig,
        parent: &DirHandle,
        file_name: &str,
    ) -> Result<ObjPath, VfsError> {
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&parent.seed, &salt)?;
        let encrypted = name::encrypt_in(config.profile(), &siv_key, &parent.dir_id, file_name)?;
        Ok(parent.physical.join(&encrypted)?)
    }

    pub async fn exists(&self, config: &VaultConfig, vault_path: &str) -> Result<bool, VfsError> {
        let (parent_segments, name) = split_parent_and_name(vault_path)?;
        let parent = match self.resolve_dir(config, &parent_segments).await {
            Ok(p) => p,
            Err(VfsError::NotFound) => return Ok(false),
            Err(e) => return Err(e),
        };
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&parent.seed, &salt)?;
        let encrypted = name::encrypt_in(config.profile(), &siv_key, &parent.dir_id, name)?;
        let target = parent.physical.join(&encrypted)?;
        Ok(self.backend.exists(&target).await?)
    }

    pub async fn quick_size_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        file_name: &str,
    ) -> Result<QuickSize, VfsError> {
        self.quick_size_of(config, &dir.0, file_name).await
    }

    async fn quick_size_of(
        &self,
        config: &VaultConfig,
        parent: &DirHandle,
        name: &str,
    ) -> Result<QuickSize, VfsError> {
        let physical = Self::resolve_file_physical(config, parent, name)?;
        let stat = self.backend.stat(&physical).await?;
        Ok(plaintext_len(stat.size))
    }

    pub async fn remove_file(
        &self,
        config: &VaultConfig,
        vault_path: &str,
    ) -> Result<(), VfsError> {
        let (parent_segments, file_name) = split_parent_and_name(vault_path)?;
        let parent = self.resolve_dir(config, &parent_segments).await?;
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&parent.seed, &salt)?;
        let encrypted = name::encrypt_in(config.profile(), &siv_key, &parent.dir_id, file_name)?;
        let target = parent.physical.join(&encrypted)?;
        if !self.backend.exists(&target).await? {
            return Err(VfsError::NotFound);
        }
        let announced = self.announce_delete(&parent, file_name).await?;
        let deleted = self.backend.delete(&target).await;
        self.observe_end(announced, deleted.is_ok()).await;
        deleted?;
        self.refresh_dirmeta(config, &parent).await;
        Ok(())
    }

    pub async fn remove_dir(&self, config: &VaultConfig, vault_path: &str) -> Result<(), VfsError> {
        let (parent_segments, dir_name) = split_parent_and_name(vault_path)?;
        let parent = self.resolve_dir(config, &parent_segments).await?;
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&parent.seed, &salt)?;
        let encrypted = name::encrypt_in(config.profile(), &siv_key, &parent.dir_id, dir_name)?;
        let link_dir = parent.physical.join(&encrypted)?;
        let (child_dir_id, seed_id) = self
            .read_dir_uvf(config, &link_dir.join(DIR_LINK_NAME)?)
            .await?;
        let child_seed = config
            .seed(seed_id)
            .cloned()
            .ok_or(FormatError::DecryptFailed)?;
        let child_physical =
            ObjPath::new(dirid::physical_path(&child_dir_id, &child_seed, &salt)?)?;
        let child_handle = DirHandle {
            dir_id: child_dir_id,
            seed: child_seed,
            physical: child_physical,
        };
        let announced = self.announce_delete(&parent, dir_name).await?;
        let removed = async {
            self.remove_dir_tree_physical(config, &child_handle).await?;
            self.backend.remove_dir_all(&link_dir).await?;
            Ok::<(), VfsError>(())
        }
        .await;
        self.observe_end(announced, removed.is_ok()).await;
        removed?;
        self.refresh_dirmeta(config, &parent).await;
        Ok(())
    }

    fn remove_dir_tree_physical<'a>(
        &'a self,
        config: &'a VaultConfig,
        dir: &'a DirHandle,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), VfsError>> + Send + 'a>>
    {
        Box::pin(async move {
            let entries = match self.list_handle(config, dir).await {
                Ok(entries) => entries,
                Err(VfsError::NotFound) => Vec::new(),
                Err(e) => return Err(e),
            };
            for entry in entries {
                if entry.kind == NodeKind::Dir {
                    let child = self.descend(config, dir, &entry.name, false).await?;
                    self.remove_dir_tree_physical(config, &child).await?;
                }
            }
            self.backend.remove_dir_all(&dir.physical).await?;
            Ok(())
        })
    }

    pub async fn rename(
        &self,
        config: &VaultConfig,
        vault_src: &str,
        vault_dest: &str,
    ) -> Result<(), VfsError> {
        let (src_parent_segments, src_name) = split_parent_and_name(vault_src)?;
        let (dst_parent_segments, dst_name) = split_parent_and_name(vault_dest)?;
        let src_parent = self.resolve_dir(config, &src_parent_segments).await?;
        let dst_parent = self.resolve_dir(config, &dst_parent_segments).await?;
        let salt = *config.kdf_salt();

        let src_siv = keys::siv_key(&src_parent.seed, &salt)?;
        let src_encrypted =
            name::encrypt_in(config.profile(), &src_siv, &src_parent.dir_id, src_name)?;
        let src_physical = src_parent.physical.join(&src_encrypted)?;

        let dst_siv = keys::siv_key(&dst_parent.seed, &salt)?;
        let dst_encrypted =
            name::encrypt_in(config.profile(), &dst_siv, &dst_parent.dir_id, dst_name)?;
        let dst_physical = dst_parent.physical.join(&dst_encrypted)?;

        let kind = self.entry_kind_at(&src_physical).await?;
        let backend_kind = match kind {
            NodeKind::File => BackendEntryKind::File,
            NodeKind::Dir => BackendEntryKind::Directory,
        };
        let announced = self
            .announce_rename(
                config,
                (&src_parent, src_name),
                (&dst_parent, dst_name),
                &src_physical,
                kind,
            )
            .await?;
        let renamed = self
            .backend
            .rename(&src_physical, &dst_physical, backend_kind)
            .await;
        self.observe_end(announced, renamed.is_ok()).await;
        renamed?;
        self.refresh_dirmeta(config, &src_parent).await;
        self.refresh_dirmeta(config, &dst_parent).await;
        Ok(())
    }

    pub async fn kind_in(
        &self,
        config: &VaultConfig,
        dir: &DirRef,
        child_name: &str,
    ) -> Result<NodeKind, VfsError> {
        let physical = Self::resolve_file_physical(config, &dir.0, child_name)?;
        self.entry_kind_at(&physical).await
    }

    async fn entry_kind_at(&self, path: &ObjPath) -> Result<NodeKind, VfsError> {
        let link_path = path.join(DIR_LINK_NAME)?;
        let (as_dir, as_file) =
            futures_util::future::join(self.backend.exists(&link_path), self.backend.exists(path))
                .await;
        if as_dir.unwrap_or(false) {
            return Ok(NodeKind::Dir);
        }
        if as_file? {
            return Ok(NodeKind::File);
        }
        Err(VfsError::NotFound)
    }

    fn root_handle(config: &VaultConfig) -> Result<DirHandle, VfsError> {
        let salt = *config.kdf_salt();
        let seed = config.initial_seed().clone();
        let dir_id = dirid::root_dir_id(&seed, &salt)?;
        let physical = ObjPath::new(dirid::physical_path(&dir_id, &seed, &salt)?)?;
        Ok(DirHandle {
            dir_id,
            seed,
            physical,
        })
    }

    async fn descend(
        &self,
        config: &VaultConfig,
        parent: &DirHandle,
        child_name: &str,
        guard: bool,
    ) -> Result<DirHandle, VfsError> {
        let salt = *config.kdf_salt();
        let siv_key = keys::siv_key(&parent.seed, &salt)?;
        let encrypted = name::encrypt_in(config.profile(), &siv_key, &parent.dir_id, child_name)?;
        let link_dir = parent.physical.join(&encrypted)?;
        let (child_dir_id, child_seed_id) = self
            .read_dir_uvf(config, &link_dir.join(DIR_LINK_NAME)?)
            .await?;
        if guard
            && let (Some(observer), Some(key)) =
                (&self.observer, self.guard_key(parent, child_name))
        {
            observer.check_dir(&key, &child_dir_id).await?;
        }
        let child_seed = config
            .seed(child_seed_id)
            .cloned()
            .ok_or(FormatError::DecryptFailed)?;
        let physical = ObjPath::new(dirid::physical_path(&child_dir_id, &child_seed, &salt)?)?;
        Ok(DirHandle {
            dir_id: child_dir_id,
            seed: child_seed,
            physical,
        })
    }

    async fn resolve_dir(
        &self,
        config: &VaultConfig,
        segments: &[&str],
    ) -> Result<DirHandle, VfsError> {
        self.resolve_dir_guarded(config, segments, true).await
    }

    async fn resolve_dir_guarded(
        &self,
        config: &VaultConfig,
        segments: &[&str],
        guard: bool,
    ) -> Result<DirHandle, VfsError> {
        let mut current = Self::root_handle(config)?;
        for segment in segments {
            current = self.descend(config, &current, segment, guard).await?;
        }
        Ok(current)
    }

    async fn resolve_or_create_dir(
        &self,
        config: &VaultConfig,
        segments: &[&str],
        track: bool,
    ) -> Result<DirHandle, VfsError> {
        let mut current = Self::root_handle(config)?;
        for segment in segments {
            current = self
                .create_child_dir(config, &current, segment, track)
                .await?;
        }
        Ok(current)
    }

    async fn create_child_dir(
        &self,
        config: &VaultConfig,
        parent: &DirHandle,
        child_name: &str,
        track: bool,
    ) -> Result<DirHandle, VfsError> {
        match self.descend(config, parent, child_name, track).await {
            Ok(handle) => return Ok(handle),
            Err(VfsError::NotFound) => {}
            Err(e) => return Err(e),
        }

        let salt = *config.kdf_salt();
        let child_dir_id = dirid::random_dir_id()?;
        let seed = config.latest_seed().clone();
        let physical = ObjPath::new(dirid::physical_path(&child_dir_id, &seed, &salt)?)?;

        let announced = if track && self.observes(&parent.dir_id) {
            self.observe_begin(vec![Op::Put {
                key: Self::entry_key(parent, child_name),
                node: Node::Dir {
                    dir_id: child_dir_id,
                },
            }])
            .await?
        } else {
            None
        };
        let created = self
            .write_dir_objects(config, parent, child_name, &child_dir_id, &physical)
            .await;
        self.observe_end(announced, matches!(created, Ok(true)))
            .await;
        if created? {
            return Ok(DirHandle {
                dir_id: child_dir_id,
                seed,
                physical,
            });
        }
        self.descend(config, parent, child_name, track).await
    }

    async fn write_dir_objects(
        &self,
        config: &VaultConfig,
        parent: &DirHandle,
        child_name: &str,
        child_dir_id: &[u8; 32],
        physical: &ObjPath,
    ) -> Result<bool, VfsError> {
        let salt = *config.kdf_salt();
        let seed_id = config.latest_seed_id();
        let seed = config.latest_seed().clone();

        self.backend.create_dir_all(physical).await?;
        self.write_node_bytes(
            config,
            &physical.join(DIR_LINK_NAME)?,
            seed_id,
            &seed,
            child_dir_id,
            keys::header_key,
            false,
            false,
        )
        .await?;

        let siv_key = keys::siv_key(&parent.seed, &salt)?;
        let encrypted = name::encrypt_in(config.profile(), &siv_key, &parent.dir_id, child_name)?;
        let link_dir = parent.physical.join(&encrypted)?;
        self.backend.create_dir_all(&link_dir).await?;

        let exclusive_supported = self.backend.capabilities().conditional_create;
        match self
            .write_node_bytes(
                config,
                &link_dir.join(DIR_LINK_NAME)?,
                seed_id,
                &seed,
                child_dir_id,
                keys::header_key,
                exclusive_supported,
                false,
            )
            .await
        {
            Ok(()) => Ok(true),
            Err(VfsError::AlreadyExists) => {
                let _ = self.backend.remove_dir_all(physical).await;
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_node_bytes(
        &self,
        config: &VaultConfig,
        dest: &ObjPath,
        seed_id: SeedId,
        seed: &Secret<32>,
        plaintext: &[u8],
        derive_header_key: HeaderKeyFn,
        exclusive: bool,
        padded: bool,
    ) -> Result<(), VfsError> {
        let padded_plaintext;
        let plaintext = if padded {
            padded_plaintext = padded_bytes(plaintext)?;
            &padded_plaintext
        } else {
            plaintext
        };
        let salt = *config.kdf_salt();
        let header_key = derive_header_key(seed, &salt)?;
        let (header_bytes, content_key, header_nonce) = header::seal(&header_key, seed_id)?;

        let mut writer = if exclusive {
            self.backend.open_write_exclusive(dest).await?
        } else {
            self.backend.open_write(dest).await?
        };
        writer.write_chunk(&header_bytes).await?;
        let mut encryptor = ContentEncryptor::new(content_key, header_nonce);
        for chunk in plaintext.chunks(PLAINTEXT_CHUNK_LEN) {
            writer.write_chunk(&encryptor.seal_next(chunk)?).await?;
        }
        if let Some(eof) = encryptor.finish()? {
            writer.write_chunk(&eof).await?;
        }
        writer.commit().await?;
        Ok(())
    }

    async fn read_node_bytes(
        &self,
        config: &VaultConfig,
        src: &ObjPath,
        derive_header_key: HeaderKeyFn,
        padded: bool,
    ) -> Result<(Vec<u8>, SeedId), VfsError> {
        let salt = *config.kdf_salt();
        let full = self.backend.get(src).await?;
        let (content_key, header_nonce, seed_id) = header::open(
            &full,
            |id| config.seed(id).cloned(),
            |seed| derive_header_key(seed, &salt),
        )?;
        let body = &full[HEADER_LEN..];
        let mut decryptor = ContentDecryptor::new(content_key, header_nonce);
        let mut plaintext = Vec::new();
        let mut offset = 0usize;
        loop {
            if decryptor.is_done() {
                break;
            }
            if offset >= body.len() {
                return Err(VfsError::Format(FormatError::DecryptFailed));
            }
            let end = (offset + CIPHERTEXT_CHUNK_LEN).min(body.len());
            plaintext.extend(decryptor.feed_chunk(&body[offset..end])?);
            offset = end;
        }
        if offset != body.len() {
            return Err(VfsError::Format(FormatError::DecryptFailed));
        }
        if padded {
            plaintext = strip_padding(plaintext)?;
        }
        Ok((plaintext, seed_id))
    }

    async fn read_dir_uvf(
        &self,
        config: &VaultConfig,
        path: &ObjPath,
    ) -> Result<([u8; 32], SeedId), VfsError> {
        let (plaintext, seed_id) = self
            .read_node_bytes(config, path, keys::header_key, false)
            .await?;
        let dir_id: [u8; 32] = plaintext.try_into().map_err(|_| {
            VfsError::Format(FormatError::Malformed("dir.uvf does not contain 32 bytes"))
        })?;
        Ok((dir_id, seed_id))
    }

    #[allow(clippy::too_many_arguments)]
    async fn seal_stream(
        &self,
        config: &VaultConfig,
        dest: &ObjPath,
        seed_id: SeedId,
        seed: &Secret<32>,
        source: &mut (dyn AsyncRead + Unpin + Send),
        content_hash: Option<[u8; 32]>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
        exclusive: bool,
        track: Option<EntryKey>,
    ) -> Result<HeaderNonce, VfsError> {
        let salt = *config.kdf_salt();

        let (header_bytes, content_key, header_nonce) = if let Some(prologue) = self
            .backend
            .load_resume_prologue(dest, content_hash)
            .await?
        {
            let (content_key, header_nonce, _seed_id) = header::open(
                &prologue,
                |id| config.seed(id).cloned(),
                |resolved_seed| keys::header_key(resolved_seed, &salt),
            )?;
            (prologue, content_key, header_nonce)
        } else {
            let header_key = keys::header_key(seed, &salt)?;
            let (header_bytes, content_key, header_nonce) = header::seal(&header_key, seed_id)?;
            self.backend
                .store_resume_prologue(dest, &header_bytes, content_hash)
                .await?;
            (header_bytes, content_key, header_nonce)
        };

        let announced = match track {
            Some(key) => {
                self.observe_begin(vec![Op::Put {
                    key,
                    node: Node::File {
                        nonce: header_nonce,
                    },
                }])
                .await?
            }
            None => None,
        };
        let written = self
            .write_sealed_body(
                dest,
                &header_bytes,
                content_key,
                header_nonce,
                source,
                on_bytes,
                exclusive,
                is_padded(config),
            )
            .await;
        self.observe_end(announced, written.is_ok()).await;
        written.map(|()| header_nonce)
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_sealed_body(
        &self,
        dest: &ObjPath,
        header_bytes: &[u8],
        content_key: ContentKey,
        header_nonce: HeaderNonce,
        source: &mut (dyn AsyncRead + Unpin + Send),
        on_bytes: &mut (dyn FnMut(u64) + Send),
        exclusive: bool,
        padded: bool,
    ) -> Result<(), VfsError> {
        let mut writer = if exclusive {
            self.backend.open_write_exclusive(dest).await?
        } else {
            self.backend.open_write(dest).await?
        };
        writer.write_chunk(header_bytes).await?;
        let mut encryptor = ContentEncryptor::new(content_key, header_nonce);

        let mut buf = vec![0u8; PLAINTEXT_CHUNK_LEN];
        let mut total = 0u64;
        loop {
            let mut filled = 0usize;
            while filled < buf.len() {
                let n = source
                    .read(&mut buf[filled..])
                    .await
                    .map_err(BackendError::from)?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            let ended = filled < buf.len();
            let sealed_now = if padded && ended {
                filled - filled % PLAINTEXT_CHUNK_LEN
            } else {
                filled
            };
            for plaintext in buf[..sealed_now].chunks(PLAINTEXT_CHUNK_LEN) {
                writer.write_chunk(&encryptor.seal_next(plaintext)?).await?;
                total += plaintext.len() as u64;
                on_bytes(total);
            }
            if ended {
                if padded {
                    let rest = &buf[sealed_now..filled];
                    let content_len = total + rest.len() as u64;
                    let stream_len = padding::padded_stream_len(content_len)
                        .ok_or(FormatError::Malformed("file too large"))?;
                    let mut remaining = stream_len - total;
                    let mut chunk = rest.to_vec();
                    while remaining > 0 {
                        let take = usize::try_from(remaining)
                            .unwrap_or(usize::MAX)
                            .min(PLAINTEXT_CHUNK_LEN);
                        chunk.resize(take, 0);
                        if take as u64 == remaining {
                            chunk[take - 8..].copy_from_slice(&content_len.to_be_bytes());
                        }
                        writer.write_chunk(&encryptor.seal_next(&chunk)?).await?;
                        remaining -= take as u64;
                        chunk.clear();
                    }
                    on_bytes(content_len);
                }
                break;
            }
            if buf.len() < SOURCE_READ_CHUNKS * PLAINTEXT_CHUNK_LEN {
                buf.resize(buf.len() * 2, 0);
            }
        }
        if let Some(eof) = encryptor.finish()? {
            writer.write_chunk(&eof).await?;
        }
        writer.commit().await?;
        Ok(())
    }

    async fn open_stream(
        &self,
        config: &VaultConfig,
        src: &ObjPath,
        sink: &mut (dyn AsyncWrite + Unpin + Send),
        on_bytes: &mut (dyn FnMut(u64) + Send),
        guard: Option<EntryKey>,
    ) -> Result<HeaderNonce, VfsError> {
        let salt = *config.kdf_salt();
        let expected = if is_padded(config) {
            let positional = self
                .open_reader_at_physical(config, src.clone(), None)
                .await?;
            Some((positional.plaintext_len, positional.header_nonce))
        } else {
            None
        };
        let mut reader = self.backend.open_read(src).await?;
        let header_bytes = reader.read_chunk(HEADER_LEN).await?;
        let (content_key, header_nonce, _seed_id) = header::open(
            &header_bytes,
            |id| config.seed(id).cloned(),
            |seed| keys::header_key(seed, &salt),
        )?;
        self.guard_file(guard.as_ref(), &header_nonce).await?;
        if expected.is_some_and(|(_, opened_nonce)| opened_nonce != header_nonce) {
            return Err(VfsError::Format(FormatError::DecryptFailed));
        }
        let content_len = expected.map(|(len, _)| len);
        let mut decryptor = ContentDecryptor::new(content_key, header_nonce);

        let mut total = 0u64;
        let mut stream_total = 0u64;
        loop {
            if decryptor.is_done() {
                break;
            }
            let chunk = reader.read_chunk(CIPHERTEXT_CHUNK_LEN).await?;
            if chunk.is_empty() {
                return Err(VfsError::Format(FormatError::DecryptFailed));
            }
            let plaintext = decryptor.feed_chunk(&chunk)?;
            stream_total += plaintext.len() as u64;
            let keep = content_len.map_or(plaintext.len(), |len| {
                usize::try_from(len - total)
                    .unwrap_or(usize::MAX)
                    .min(plaintext.len())
            });
            if keep > 0 {
                sink.write_all(&plaintext[..keep])
                    .await
                    .map_err(BackendError::from)?;
                total += keep as u64;
                on_bytes(total);
            }
        }
        let trailing = reader.read_chunk(1).await?;
        if !trailing.is_empty() {
            return Err(VfsError::Format(FormatError::DecryptFailed));
        }
        if content_len.is_some_and(|len| padding::padded_stream_len(len) != Some(stream_total)) {
            return Err(VfsError::Format(FormatError::DecryptFailed));
        }
        sink.flush().await.map_err(BackendError::from)?;
        Ok(header_nonce)
    }

    async fn try_read_dirmeta_cache(
        &self,
        config: &VaultConfig,
        dir: &DirHandle,
    ) -> Result<HashMap<String, dirmeta::Entry>, VfsError> {
        let path = dir.physical.join(DIRMETA_NAME)?;
        let (plaintext, _seed_id) = self
            .read_node_bytes(config, &path, keys::dirmeta_header_key, is_padded(config))
            .await?;
        let entries = dirmeta::decode(&plaintext)?;
        Ok(entries
            .into_iter()
            .map(|e| (e.encrypted_name.clone(), e))
            .collect())
    }

    async fn refresh_dirmeta(&self, config: &VaultConfig, dir: &DirHandle) {
        {
            let mut guard = self.pending_dirmeta.lock().expect("mutex poisoned");
            if let Some(pending) = guard.as_mut() {
                pending.insert(dir.physical.as_str().to_string(), dir.clone());
                return;
            }
        }
        self.refresh_dirmeta_now(config, dir).await;
    }

    async fn refresh_dirmeta_now(&self, config: &VaultConfig, dir: &DirHandle) {
        let Ok(mut scan) = self.scan_dir(config, dir).await else {
            return;
        };
        if is_padded(config) && config.decoy_entries() {
            self.rebalance_decoys(config, dir, &mut scan).await;
        }
        let decoy_entries = scan.decoys.into_iter().map(|e| dirmeta::Entry {
            name: e.name,
            encrypted_name: e.encrypted_name,
            kind: dirmeta::NodeKind::Decoy,
            dir_id: None,
        });
        let raw_entries: Vec<dirmeta::Entry> = scan
            .entries
            .into_iter()
            .map(|e| dirmeta::Entry {
                name: e.name,
                encrypted_name: e.encrypted_name,
                kind: match e.kind {
                    NodeKind::File => dirmeta::NodeKind::File,
                    NodeKind::Dir => dirmeta::NodeKind::Dir,
                },
                dir_id: e.dir_id,
            })
            .chain(decoy_entries)
            .collect();
        let plaintext = dirmeta::encode(&raw_entries);
        let seed_id = config.latest_seed_id();
        let seed = config.latest_seed().clone();
        let Ok(dest) = dir.physical.join(DIRMETA_NAME) else {
            return;
        };
        let _ = self
            .write_node_bytes(
                config,
                &dest,
                seed_id,
                &seed,
                &plaintext,
                keys::dirmeta_header_key,
                false,
                is_padded(config),
            )
            .await;
    }
}

fn split_logical_path(path: &str) -> Result<Vec<&str>, VfsError> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut segments = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(VfsError::InvalidName(path.to_string()));
        }
        segments.push(segment);
    }
    Ok(segments)
}

fn split_parent_and_name(path: &str) -> Result<(Vec<&str>, &str), VfsError> {
    let segments = split_logical_path(path)?;
    match segments.split_last() {
        Some((name, parent)) => Ok((parent.to_vec(), name)),
        None => Err(VfsError::InvalidName("empty path".to_string())),
    }
}
