// SPDX-License-Identifier: GPL-3.0-or-later

use cu_format::config::VaultConfig;
use cu_format::manifest::{self, MANIFEST_DIR, MANIFEST_PATH, Writer};
use cu_format::manifest_v2::{
    self as v2, Body, CHECKPOINT_AFTER_DELTA_BYTES, CHECKPOINT_EVERY_DELTAS, ChainError, EntryKey,
    Head, ManifestV2, Node, Op, State,
};
use cu_index::ManifestHeadRow;
use cu_vfs::{DirRef, VaultFs, VfsError};
use futures_util::stream::{self, StreamExt};

use cu_msg::Msg;

use crate::error::SyncError;
use crate::manifest::{HeadStores, ManifestKeys, load_head, store_head};

const MAX_RELIST_ATTEMPTS: u32 = 3;
const READ_CONCURRENCY: usize = 8;
const MAX_APPEND_ATTEMPTS: u32 = 8;

#[derive(Debug, Clone)]
pub(crate) struct Loaded {
    pub(crate) state: State,
    pub(crate) head: Head,
    pub(crate) head_mac: [u8; 32],
    pub(crate) head_device: [u8; 16],
    pub(crate) head_updated_ms: i64,
    pub(crate) last_checkpoint: i64,
    pub(crate) bytes_since_checkpoint: usize,
    pub(crate) last_checkpoint_len: usize,
}

impl Loaded {
    pub(crate) fn head_row(&self, vault_id: [u8; 32]) -> ManifestHeadRow {
        ManifestHeadRow {
            vault_id,
            seq: self.head.seq,
            payload_hash: self.head.payload_hash,
            mac: self.head_mac.to_vec(),
            device_id: self.head_device,
            updated_at: self.head_updated_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum LogRead {
    Empty,
    Legacy { seq: i64, payload_hash: [u8; 32] },
    V2(Box<Loaded>),
}

impl LogRead {
    fn head(&self) -> Option<(i64, [u8; 32])> {
        match self {
            Self::Empty => None,
            Self::Legacy { seq, payload_hash } => Some((*seq, *payload_hash)),
            Self::V2(l) => Some((l.head.seq, l.head.payload_hash)),
        }
    }
}

fn integrity(msg: Msg) -> SyncError {
    SyncError::IntegrityFailure(msg)
}

fn chain_error(e: ChainError) -> SyncError {
    integrity(match e {
        ChainError::Empty => Msg::new("MANIFEST_CHAIN_EMPTY"),
        ChainError::FirstNotCheckpoint => Msg::new("MANIFEST_CHAIN_FIRST"),
        ChainError::SeqGap { expected, found } => {
            Msg::new("MANIFEST_CHAIN_GAP").arg(expected).arg(found)
        }
        ChainError::BrokenLink { seq } => Msg::new("MANIFEST_CHAIN_LINK").arg(seq),
        ChainError::BadCheckpointPointer { seq } => Msg::new("MANIFEST_CHAIN_CHECKPOINT").arg(seq),
    })
}

pub(crate) fn check_head(
    stores: &HeadStores<'_>,
    vault_id: &[u8; 32],
    read: &LogRead,
) -> Result<(), SyncError> {
    let local = load_head(stores, vault_id)?;
    match (read.head(), local) {
        (Some((seq, hash)), Some(local)) => {
            if seq < local.seq {
                return Err(integrity(
                    Msg::new("MANIFEST_ROLLBACK").arg(seq).arg(local.seq),
                ));
            }
            if seq == local.seq && hash != local.payload_hash {
                return Err(integrity(Msg::new("MANIFEST_SAME_SEQ").arg(seq)));
            }
            Ok(())
        }
        (None, Some(local)) => Err(integrity(Msg::new("MANIFEST_VANISHED").arg(local.seq))),
        (_, None) => Ok(()),
    }
}

pub(crate) async fn check_run_start(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    stores: &mut HeadStores<'_>,
) -> Result<(), SyncError> {
    let keys = ManifestKeys::derive(config)?;
    let read = read_log(vault_fs, config, &keys).await?;
    check_head(stores, &keys.vault_id, &read)?;
    if let LogRead::V2(loaded) = &read {
        store_head(stores, &loaded.head_row(keys.vault_id))?;
    }
    Ok(())
}

struct ObjectRead {
    object: ManifestV2,
    mac: [u8; 32],
    len: usize,
}

enum ObjectError {
    Missing,
    Legacy(i64, [u8; 32]),
    Failed(SyncError),
}

impl From<SyncError> for ObjectError {
    fn from(e: SyncError) -> Self {
        Self::Failed(e)
    }
}

async fn read_object(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    keys: &ManifestKeys,
    dir: &DirRef,
    seq: i64,
) -> Result<ObjectRead, ObjectError> {
    let name = seq.to_string();
    let mut buf = Vec::new();
    match vault_fs
        .read_internal_in(config, dir, &name, &mut buf)
        .await
    {
        Ok(()) => {}
        Err(VfsError::NotFound) => return Err(ObjectError::Missing),
        Err(e) => return Err(ObjectError::Failed(e.into())),
    }
    match v2::peek_version(&buf) {
        Some(manifest::MANIFEST_VERSION) => {
            let (m, tag) = manifest::decode(&buf).map_err(SyncError::from)?;
            if !manifest::verify_mac(keys.mac_key(), &m, &tag) {
                return Err(bad_mac().into());
            }
            Err(ObjectError::Legacy(m.seq, manifest::payload_hash(&m)))
        }
        Some(v2::MANIFEST_V2_VERSION) => {
            let (object, tag) = v2::decode(&buf).map_err(SyncError::from)?;
            if !v2::verify_mac(keys.mac_key(), &object, &tag) {
                return Err(bad_mac().into());
            }
            if object.seq != seq {
                return Err(
                    integrity(Msg::new("MANIFEST_SEQ_MISMATCH").arg(seq).arg(object.seq)).into(),
                );
            }
            Ok(ObjectRead {
                object,
                mac: tag,
                len: buf.len(),
            })
        }
        other => {
            Err(integrity(Msg::new("MANIFEST_UNKNOWN_VERSION").arg(format!("{other:?}"))).into())
        }
    }
}

fn bad_mac() -> SyncError {
    integrity(Msg::new("MANIFEST_BAD_MAC"))
}

async fn read_legacy_file(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    keys: &ManifestKeys,
) -> Result<LogRead, SyncError> {
    let mut buf = Vec::new();
    match vault_fs
        .read_internal_file(config, MANIFEST_PATH, &mut buf)
        .await
    {
        Ok(()) => {
            let (m, tag) = manifest::decode(&buf)?;
            if !manifest::verify_mac(keys.mac_key(), &m, &tag) {
                return Err(bad_mac());
            }
            Ok(LogRead::Legacy {
                seq: m.seq,
                payload_hash: manifest::payload_hash(&m),
            })
        }
        Err(VfsError::NotFound) => Ok(LogRead::Empty),
        Err(e) => Err(e.into()),
    }
}

pub(crate) async fn read_log(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    keys: &ManifestKeys,
) -> Result<LogRead, SyncError> {
    let dir = match vault_fs
        .resolve_dir_ref_untracked(config, MANIFEST_DIR)
        .await
    {
        Ok(dir) => dir,
        Err(VfsError::NotFound) => return read_legacy_file(vault_fs, config, keys).await,
        Err(e) => return Err(e.into()),
    };
    for attempt in 1..=MAX_RELIST_ATTEMPTS {
        let highest = vault_fs
            .list_in(config, &dir)
            .await?
            .iter()
            .filter_map(|e| manifest::manifest_seq_from_name(&e.name))
            .max();
        let Some(highest) = highest else {
            return read_legacy_file(vault_fs, config, keys).await;
        };
        match read_chain_at(vault_fs, config, keys, &dir, highest).await {
            Ok(read) => return Ok(read),
            Err(ObjectError::Missing) if attempt < MAX_RELIST_ATTEMPTS => {}
            Err(ObjectError::Missing) => {
                return Err(integrity(
                    Msg::new("MANIFEST_OBJECT_VANISHED")
                        .arg(highest)
                        .arg(MAX_RELIST_ATTEMPTS),
                ));
            }
            Err(ObjectError::Legacy(seq, payload_hash)) => {
                return Ok(LogRead::Legacy { seq, payload_hash });
            }
            Err(ObjectError::Failed(e)) => return Err(e),
        }
    }
    unreachable!("the loop above always returns within {MAX_RELIST_ATTEMPTS} attempts")
}

async fn read_chain_at(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    keys: &ManifestKeys,
    dir: &DirRef,
    highest: i64,
) -> Result<LogRead, ObjectError> {
    let newest = read_object(vault_fs, config, keys, dir, highest).await?;
    let cp = newest.object.cp;
    let mut chain: Vec<ManifestV2> = Vec::new();
    let mut bytes = if matches!(newest.object.body, Body::Delta { .. }) {
        newest.len
    } else {
        0
    };
    let mut checkpoint_len = newest.len;
    if matches!(newest.object.body, Body::Delta { .. }) {
        if cp < 1 || cp >= highest {
            return Err(integrity(Msg::new("MANIFEST_BAD_CHECKPOINT").arg(highest).arg(cp)).into());
        }
        let results: Vec<Result<ObjectRead, ObjectError>> = stream::iter(cp..highest)
            .map(|seq| read_object(vault_fs, config, keys, dir, seq))
            .buffered(READ_CONCURRENCY)
            .collect()
            .await;
        for r in results {
            let read = match r {
                Ok(read) => read,
                Err(ObjectError::Legacy(seq, _)) => {
                    return Err(integrity(Msg::new("MANIFEST_V1_IN_V2").arg(seq)).into());
                }
                Err(e) => return Err(e),
            };
            if matches!(read.object.body, Body::Delta { .. }) {
                bytes += read.len;
            } else {
                checkpoint_len = read.len;
            }
            chain.push(read.object);
        }
    }
    chain.push(newest.object.clone());
    let (state, head) = v2::replay(&chain).map_err(chain_error)?;
    debug_assert_eq!(head.seq, highest);
    Ok(LogRead::V2(Box::new(Loaded {
        state,
        head,
        head_mac: newest.mac,
        head_device: crate::hexutil::decode(&newest.object.last_writer.device_id)
            .try_into()
            .unwrap_or([0u8; 16]),
        head_updated_ms: newest.object.updated_at_ms,
        last_checkpoint: newest.object.cp,
        bytes_since_checkpoint: bytes,
        last_checkpoint_len: checkpoint_len,
    })))
}

fn extend(current: LogRead, winner: ObjectRead) -> Result<LogRead, SyncError> {
    let expected_prev = current.head().map_or([0u8; 32], |(_, hash)| hash);
    extend_from(current, expected_prev, winner)
}

fn extend_from(
    current: LogRead,
    expected_prev: [u8; 32],
    winner: ObjectRead,
) -> Result<LogRead, SyncError> {
    let ObjectRead { object, mac, len } = winner;
    let device: [u8; 16] = crate::hexutil::decode(&object.last_writer.device_id)
        .try_into()
        .unwrap_or([0u8; 16]);
    let hash = v2::payload_hash(&object);
    if object.prev != expected_prev {
        return Err(integrity(Msg::new("MANIFEST_FORK")));
    }
    match object.body {
        Body::Checkpoint { coverage, entries } => Ok(LogRead::V2(Box::new(Loaded {
            state: State::from_checkpoint(coverage, &entries),
            head: Head {
                seq: object.seq,
                payload_hash: hash,
            },
            head_mac: mac,
            head_device: device,
            head_updated_ms: object.updated_at_ms,
            last_checkpoint: object.seq,
            bytes_since_checkpoint: 0,
            last_checkpoint_len: len,
        }))),
        Body::Delta { ops } => {
            let LogRead::V2(mut loaded) = current else {
                return Err(integrity(Msg::new("MANIFEST_DELTA_FIRST")));
            };
            loaded.state.apply(&ops);
            loaded.head = Head {
                seq: object.seq,
                payload_hash: hash,
            };
            loaded.head_mac = mac;
            loaded.head_device = device;
            loaded.head_updated_ms = object.updated_at_ms;
            loaded.bytes_since_checkpoint += len;
            Ok(LogRead::V2(loaded))
        }
    }
}

pub(crate) struct LogCtx<'a> {
    pub(crate) vault_fs: &'a VaultFs,
    pub(crate) config: &'a VaultConfig,
    pub(crate) keys: &'a ManifestKeys,
}

struct NextObject {
    object: ManifestV2,
    checkpoint_state: Option<State>,
    prune_from: Option<i64>,
}

const SYSTEM_DIR: &str = ".cryptoupload";

fn with_system_dir(
    current: &LogRead,
    keys: &ManifestKeys,
    system: [u8; 32],
    ops: &[Op],
) -> Vec<Op> {
    let key = EntryKey {
        parent: keys.vault_id,
        name: SYSTEM_DIR.to_string(),
    };
    let node = Node::Dir { dir_id: system };
    let known = match current {
        LogRead::V2(l) => l
            .state
            .slots
            .get(&key)
            .is_some_and(|s| s.current.as_ref() == Some(&node)),
        LogRead::Empty | LogRead::Legacy { .. } => false,
    };
    let mut full = Vec::with_capacity(ops.len() + 2);
    if !known {
        full.push(Op::Put {
            key: key.clone(),
            node: node.clone(),
        });
        full.push(Op::Confirm {
            key,
            node: Some(node),
        });
    }
    full.extend_from_slice(ops);
    full
}

fn next_object(
    keys: &ManifestKeys,
    current: &LogRead,
    writer: &Writer,
    ops: &[Op],
    now_ms: i64,
    force_coverage: Option<bool>,
) -> NextObject {
    let (seq, prev) = match current {
        LogRead::Empty => (1, [0u8; 32]),
        LogRead::Legacy { seq, payload_hash } => (seq + 1, *payload_hash),
        LogRead::V2(l) => (l.head.seq + 1, l.head.payload_hash),
    };
    let write_checkpoint = force_coverage.is_some()
        || match current {
            LogRead::V2(l) => {
                u32::try_from(seq - 1 - l.last_checkpoint).unwrap_or(u32::MAX)
                    >= CHECKPOINT_EVERY_DELTAS
                    || l.bytes_since_checkpoint
                        >= CHECKPOINT_AFTER_DELTA_BYTES.max(l.last_checkpoint_len)
            }
            LogRead::Empty | LogRead::Legacy { .. } => true,
        };
    let prune_from = match current {
        LogRead::V2(l) => Some(l.last_checkpoint),
        LogRead::Legacy { seq, .. } => Some(*seq),
        LogRead::Empty => None,
    };
    let (body, checkpoint_state, cp) = if write_checkpoint {
        let mut state = match current {
            LogRead::V2(l) => l.state.clone(),
            LogRead::Empty | LogRead::Legacy { .. } => State::default(),
        };
        state.apply(ops);
        state.prune_unreachable(&keys.vault_id);
        if let Some(coverage) = force_coverage {
            state.coverage = coverage;
        }
        let body = Body::Checkpoint {
            coverage: state.coverage,
            entries: state.to_checkpoint_entries(),
        };
        (body, Some(state), seq)
    } else {
        let cp = match current {
            LogRead::V2(l) => l.last_checkpoint,
            LogRead::Empty | LogRead::Legacy { .. } => seq,
        };
        (Body::Delta { ops: ops.to_vec() }, None, cp)
    };
    NextObject {
        object: ManifestV2 {
            seq,
            updated_at_ms: now_ms,
            last_writer: writer.clone(),
            prev,
            cp,
            body,
        },
        checkpoint_state,
        prune_from,
    }
}

pub(crate) async fn append_ops(
    ctx: &LogCtx<'_>,
    stores: &mut HeadStores<'_>,
    writer: &Writer,
    ops: &[Op],
    now_ms: i64,
    base: LogRead,
    force_coverage: Option<bool>,
) -> Result<Loaded, SyncError> {
    let LogCtx {
        vault_fs,
        config,
        keys,
    } = *ctx;
    let dir = vault_fs
        .resolve_or_create_dir_ref_untracked(config, MANIFEST_DIR)
        .await?;
    let system = vault_fs
        .resolve_dir_ref_untracked(config, SYSTEM_DIR)
        .await?
        .dir_id();
    let exclusive = vault_fs.backend().capabilities().conditional_create;
    let mut current = base;

    for attempt in 1..=MAX_APPEND_ATTEMPTS {
        let full_ops = with_system_dir(&current, keys, system, ops);
        let ops = full_ops.as_slice();
        let next = next_object(keys, &current, writer, ops, now_ms, force_coverage);
        let seq = next.object.seq;
        let tag = v2::mac(keys.mac_key(), &next.object);
        let bytes = v2::encode(&next.object, &tag);
        let encoded_len = bytes.len();
        let name = seq.to_string();
        let mut cursor = std::io::Cursor::new(bytes);

        let written = if exclusive {
            vault_fs
                .write_file_if_absent_in(config, &dir, &name, &mut cursor, None, &mut |_| {})
                .await
        } else {
            vault_fs
                .write_file_with_nonce_in(config, &dir, &name, &mut cursor, None, &mut |_| {})
                .await
                .map(|_| ())
        };

        match written {
            Ok(()) => {
                let is_checkpoint = next.checkpoint_state.is_some();
                let prune_from = next.prune_from;
                let loaded = finish_append(current, next, tag, encoded_len, ops);
                store_head(stores, &loaded.head_row(keys.vault_id))?;
                if let (true, Some(keep_from)) = (is_checkpoint, prune_from) {
                    prune_before(vault_fs, config, &dir, keep_from).await;
                }
                return Ok(loaded);
            }
            Err(VfsError::AlreadyExists) => {
                if attempt == MAX_APPEND_ATTEMPTS {
                    return Err(integrity(
                        Msg::new("MANIFEST_APPEND_CONTENTION").arg(MAX_APPEND_ATTEMPTS),
                    ));
                }
                if attempt >= 3 {
                    current = read_log(vault_fs, config, keys).await?;
                    continue;
                }
                let winner = match read_object(vault_fs, config, keys, &dir, seq).await {
                    Ok(w) => w,
                    Err(ObjectError::Failed(e)) => return Err(e),
                    Err(ObjectError::Missing | ObjectError::Legacy(..)) => {
                        return Err(integrity(Msg::new("MANIFEST_WINNER_UNREADABLE")));
                    }
                };
                current = extend(current, winner)?;
            }
            Err(e) => return Err(e.into()),
        }
    }
    unreachable!("the loop above returns or exhausts {MAX_APPEND_ATTEMPTS} attempts")
}

fn finish_append(
    current: LogRead,
    next: NextObject,
    mac: [u8; 32],
    encoded_len: usize,
    ops: &[Op],
) -> Loaded {
    let object = &next.object;
    let seq = object.seq;
    let payload_hash = v2::payload_hash(object);
    let device: [u8; 16] = crate::hexutil::decode(&object.last_writer.device_id)
        .try_into()
        .unwrap_or([0u8; 16]);
    let head = Head { seq, payload_hash };
    if let Some(state) = next.checkpoint_state {
        return Loaded {
            state,
            head,
            head_mac: mac,
            head_device: device,
            head_updated_ms: object.updated_at_ms,
            last_checkpoint: seq,
            bytes_since_checkpoint: 0,
            last_checkpoint_len: encoded_len,
        };
    }
    let LogRead::V2(mut loaded) = current else {
        unreachable!("a delta is only written on top of a v2 log")
    };
    loaded.state.apply(ops);
    loaded.head = head;
    loaded.head_mac = mac;
    loaded.head_device = device;
    loaded.head_updated_ms = object.updated_at_ms;
    loaded.bytes_since_checkpoint += encoded_len;
    *loaded
}

async fn prune_before(vault_fs: &VaultFs, config: &VaultConfig, dir: &DirRef, keep_from: i64) {
    let Ok(entries) = vault_fs.list_in(config, dir).await else {
        return;
    };
    for entry in entries {
        if let Some(seq) = manifest::manifest_seq_from_name(&entry.name)
            && seq < keep_from
        {
            let _ = vault_fs.remove_file_in(config, dir, &entry.name).await;
        }
    }
}
