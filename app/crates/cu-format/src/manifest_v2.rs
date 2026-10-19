// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};

use cu_crypto::aead::NONCE_LEN;
use serde::{Deserialize, Serialize};

use crate::error::FormatError;
use crate::manifest::{Writer, hex_decode_n, hex_encode};

pub const MANIFEST_V2_VERSION: u32 = 2;

pub const CHECKPOINT_EVERY_DELTAS: u32 = 256;
pub const CHECKPOINT_AFTER_DELTA_BYTES: usize = 1 << 20;

pub const MAX_PENDING_PER_ENTRY: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryKey {
    pub parent: [u8; 32],
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
    File { nonce: [u8; NONCE_LEN] },
    Dir { dir_id: [u8; 32] },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Slot {
    pub current: Option<Node>,
    pub pending: Vec<Option<Node>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Put {
        key: EntryKey,
        node: Node,
    },
    Del {
        key: EntryKey,
    },
    Confirm {
        key: EntryKey,
        node: Option<Node>,
    },
    Abort {
        key: EntryKey,
        attempted: Option<Node>,
    },
}

impl Op {
    #[must_use]
    pub fn key(&self) -> &EntryKey {
        match self {
            Self::Put { key, .. }
            | Self::Del { key }
            | Self::Confirm { key, .. }
            | Self::Abort { key, .. } => key,
        }
    }

    #[must_use]
    pub fn confirmation(&self) -> Option<Self> {
        match self {
            Self::Put { key, node } => Some(Self::Confirm {
                key: key.clone(),
                node: Some(node.clone()),
            }),
            Self::Del { key } => Some(Self::Confirm {
                key: key.clone(),
                node: None,
            }),
            Self::Confirm { .. } | Self::Abort { .. } => None,
        }
    }

    #[must_use]
    pub fn abortion(&self) -> Option<Self> {
        match self {
            Self::Put { key, node } => Some(Self::Abort {
                key: key.clone(),
                attempted: Some(node.clone()),
            }),
            Self::Del { key } => Some(Self::Abort {
                key: key.clone(),
                attempted: None,
            }),
            Self::Confirm { .. } | Self::Abort { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointEntry {
    pub key: EntryKey,
    pub slot: Slot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Checkpoint {
        coverage: bool,
        entries: Vec<CheckpointEntry>,
    },
    Delta {
        ops: Vec<Op>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestV2 {
    pub seq: i64,
    pub updated_at_ms: i64,
    pub last_writer: Writer,
    pub prev: [u8; 32],
    pub cp: i64,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    pub slots: BTreeMap<EntryKey, Slot>,
    pub coverage: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acceptance {
    Current,
    Pending,
    Unknown,
    Mismatch,
}

impl State {
    pub fn apply(&mut self, ops: &[Op]) {
        for op in ops {
            match op {
                Op::Put { key, node } => {
                    let slot = self.slots.entry(key.clone()).or_default();
                    if slot.current.as_ref() != Some(node) {
                        let old = slot.current.take();
                        push_pending(&mut slot.pending, old);
                        slot.current = Some(node.clone());
                    }
                }
                Op::Del { key } => {
                    if let Some(slot) = self.slots.get_mut(key)
                        && let Some(old) = slot.current.take()
                    {
                        push_pending(&mut slot.pending, Some(old));
                    }
                }
                Op::Confirm { key, node } => {
                    if let Some(slot) = self.slots.get_mut(key)
                        && slot.current == *node
                    {
                        slot.pending.clear();
                        if slot.current.is_none() {
                            self.slots.remove(key);
                        }
                    }
                }
                Op::Abort { key, attempted } => {
                    if let Some(slot) = self.slots.get_mut(key)
                        && slot.current == *attempted
                    {
                        slot.current = slot.pending.pop().flatten();
                        if slot.current.is_none() && slot.pending.is_empty() {
                            self.slots.remove(key);
                        }
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn judge(&self, key: &EntryKey, served: Option<&Node>) -> Acceptance {
        let Some(slot) = self.slots.get(key) else {
            return match served {
                None => Acceptance::Current,
                Some(_) => Acceptance::Unknown,
            };
        };
        if slot.current.as_ref() == served {
            Acceptance::Current
        } else if slot.pending.iter().any(|p| p.as_ref() == served) {
            Acceptance::Pending
        } else {
            Acceptance::Mismatch
        }
    }

    pub fn prune_unreachable(&mut self, root: &[u8; 32]) {
        let mut subdirs: BTreeMap<[u8; 32], Vec<[u8; 32]>> = BTreeMap::new();
        for (key, slot) in &self.slots {
            for node in slot.current.iter().chain(slot.pending.iter().flatten()) {
                if let Node::Dir { dir_id } = node {
                    subdirs.entry(key.parent).or_default().push(*dir_id);
                }
            }
        }
        let mut reachable: BTreeSet<[u8; 32]> = BTreeSet::from([*root]);
        let mut to_visit = vec![*root];
        while let Some(dir) = to_visit.pop() {
            for child in subdirs.get(&dir).into_iter().flatten() {
                if reachable.insert(*child) {
                    to_visit.push(*child);
                }
            }
        }
        self.slots.retain(|key, _| reachable.contains(&key.parent));
    }

    #[must_use]
    pub fn to_checkpoint_entries(&self) -> Vec<CheckpointEntry> {
        self.slots
            .iter()
            .map(|(key, slot)| CheckpointEntry {
                key: key.clone(),
                slot: slot.clone(),
            })
            .collect()
    }

    #[must_use]
    pub fn from_checkpoint(coverage: bool, entries: &[CheckpointEntry]) -> Self {
        Self {
            slots: entries
                .iter()
                .map(|e| (e.key.clone(), e.slot.clone()))
                .collect(),
            coverage,
        }
    }
}

fn push_pending(pending: &mut Vec<Option<Node>>, node: Option<Node>) {
    if pending.contains(&node) {
        return;
    }
    pending.push(node);
    if pending.len() > MAX_PENDING_PER_ENTRY {
        pending.remove(0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
    Empty,
    FirstNotCheckpoint,
    SeqGap { expected: i64, found: i64 },
    BrokenLink { seq: i64 },
    BadCheckpointPointer { seq: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Head {
    pub seq: i64,
    pub payload_hash: [u8; 32],
}

pub fn replay(objects: &[ManifestV2]) -> Result<(State, Head), ChainError> {
    let (first, rest) = objects.split_first().ok_or(ChainError::Empty)?;
    let Body::Checkpoint { coverage, entries } = &first.body else {
        return Err(ChainError::FirstNotCheckpoint);
    };
    if first.cp != first.seq {
        return Err(ChainError::BadCheckpointPointer { seq: first.seq });
    }
    let mut state = State::from_checkpoint(*coverage, entries);
    let mut head = Head {
        seq: first.seq,
        payload_hash: payload_hash(first),
    };
    let mut last_checkpoint = first.seq;
    for obj in rest {
        let expected = head.seq.checked_add(1).ok_or(ChainError::SeqGap {
            expected: i64::MAX,
            found: obj.seq,
        })?;
        if obj.seq != expected {
            return Err(ChainError::SeqGap {
                expected,
                found: obj.seq,
            });
        }
        if obj.prev != head.payload_hash {
            return Err(ChainError::BrokenLink { seq: obj.seq });
        }
        match &obj.body {
            Body::Checkpoint { coverage, entries } => {
                state = State::from_checkpoint(*coverage, entries);
                last_checkpoint = obj.seq;
            }
            Body::Delta { ops } => state.apply(ops),
        }
        if obj.cp != last_checkpoint {
            return Err(ChainError::BadCheckpointPointer { seq: obj.seq });
        }
        head = Head {
            seq: obj.seq,
            payload_hash: payload_hash(obj),
        };
    }
    Ok((state, head))
}

#[derive(Serialize, Deserialize)]
struct WireNode {
    t: String,
    v: String,
}

#[derive(Serialize, Deserialize)]
struct WireKey {
    p: String,
    n: String,
}

#[derive(Serialize, Deserialize)]
struct WireSlot {
    #[serde(flatten)]
    key: WireKey,
    c: Option<WireNode>,
    w: Vec<Option<WireNode>>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum WireOp {
    Put {
        #[serde(flatten)]
        key: WireKey,
        node: WireNode,
    },
    Del {
        #[serde(flatten)]
        key: WireKey,
    },
    Confirm {
        #[serde(flatten)]
        key: WireKey,
        node: Option<WireNode>,
    },
    Abort {
        #[serde(flatten)]
        key: WireKey,
        attempted: Option<WireNode>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum WireBody {
    Checkpoint {
        coverage: bool,
        entries: Vec<WireSlot>,
    },
    Delta {
        ops: Vec<WireOp>,
    },
}

#[derive(Serialize, Deserialize)]
struct WireWriter {
    #[serde(rename = "deviceId")]
    device_id: String,
    label: String,
}

#[derive(Serialize, Deserialize)]
struct WireManifest {
    #[serde(rename = "cryptoUploadManifestVersion")]
    version: u32,
    seq: i64,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
    #[serde(rename = "lastWriter")]
    last_writer: WireWriter,
    prev: String,
    cp: i64,
    body: WireBody,
}

fn node_to_wire(n: &Node) -> WireNode {
    match n {
        Node::File { nonce } => WireNode {
            t: "f".to_string(),
            v: hex_encode(nonce),
        },
        Node::Dir { dir_id } => WireNode {
            t: "d".to_string(),
            v: hex_encode(dir_id),
        },
    }
}

fn node_from_wire(w: &WireNode) -> Result<Node, FormatError> {
    match w.t.as_str() {
        "f" => hex_decode_n::<NONCE_LEN>(&w.v)
            .map(|nonce| Node::File { nonce })
            .ok_or(FormatError::Malformed(
                "manifest v2 entry nonce is not hex of the expected length",
            )),
        "d" => hex_decode_n::<32>(&w.v)
            .map(|dir_id| Node::Dir { dir_id })
            .ok_or(FormatError::Malformed(
                "manifest v2 entry dirId is not 32-byte hex",
            )),
        _ => Err(FormatError::Malformed("unknown manifest v2 node kind")),
    }
}

fn opt_node_to_wire(n: Option<&Node>) -> Option<WireNode> {
    n.map(node_to_wire)
}

fn opt_node_from_wire(w: Option<&WireNode>) -> Result<Option<Node>, FormatError> {
    w.map(node_from_wire).transpose()
}

fn key_to_wire(k: &EntryKey) -> WireKey {
    WireKey {
        p: hex_encode(&k.parent),
        n: k.name.clone(),
    }
}

fn key_from_wire(w: &WireKey) -> Result<EntryKey, FormatError> {
    let parent = hex_decode_n::<32>(&w.p).ok_or(FormatError::Malformed(
        "manifest v2 entry parent dirId is not 32-byte hex",
    ))?;
    if w.n.is_empty() || w.n.contains('/') {
        return Err(FormatError::Malformed(
            "manifest v2 entry name is empty or contains '/'",
        ));
    }
    Ok(EntryKey {
        parent,
        name: w.n.clone(),
    })
}

fn op_to_wire(op: &Op) -> WireOp {
    match op {
        Op::Put { key, node } => WireOp::Put {
            key: key_to_wire(key),
            node: node_to_wire(node),
        },
        Op::Del { key } => WireOp::Del {
            key: key_to_wire(key),
        },
        Op::Confirm { key, node } => WireOp::Confirm {
            key: key_to_wire(key),
            node: opt_node_to_wire(node.as_ref()),
        },
        Op::Abort { key, attempted } => WireOp::Abort {
            key: key_to_wire(key),
            attempted: opt_node_to_wire(attempted.as_ref()),
        },
    }
}

fn op_from_wire(w: &WireOp) -> Result<Op, FormatError> {
    Ok(match w {
        WireOp::Put { key, node } => Op::Put {
            key: key_from_wire(key)?,
            node: node_from_wire(node)?,
        },
        WireOp::Del { key } => Op::Del {
            key: key_from_wire(key)?,
        },
        WireOp::Confirm { key, node } => Op::Confirm {
            key: key_from_wire(key)?,
            node: opt_node_from_wire(node.as_ref())?,
        },
        WireOp::Abort { key, attempted } => Op::Abort {
            key: key_from_wire(key)?,
            attempted: opt_node_from_wire(attempted.as_ref())?,
        },
    })
}

fn to_wire(m: &ManifestV2) -> WireManifest {
    let body = match &m.body {
        Body::Checkpoint { coverage, entries } => WireBody::Checkpoint {
            coverage: *coverage,
            entries: entries
                .iter()
                .map(|e| WireSlot {
                    key: key_to_wire(&e.key),
                    c: opt_node_to_wire(e.slot.current.as_ref()),
                    w: e.slot
                        .pending
                        .iter()
                        .map(|p| opt_node_to_wire(p.as_ref()))
                        .collect(),
                })
                .collect(),
        },
        Body::Delta { ops } => WireBody::Delta {
            ops: ops.iter().map(op_to_wire).collect(),
        },
    };
    WireManifest {
        version: MANIFEST_V2_VERSION,
        seq: m.seq,
        updated_at_ms: m.updated_at_ms,
        last_writer: WireWriter {
            device_id: m.last_writer.device_id.clone(),
            label: m.last_writer.label.clone(),
        },
        prev: hex_encode(&m.prev),
        cp: m.cp,
        body,
    }
}

fn canonical_bytes(m: &ManifestV2) -> Vec<u8> {
    #[allow(clippy::expect_used)]
    let out = serde_json::to_vec(&to_wire(m)).expect("manifest_v2 is always serializable");
    out
}

#[must_use]
pub fn payload_hash(m: &ManifestV2) -> [u8; 32] {
    let mut hasher = cu_crypto::hash::Hasher::new();
    hasher.update(&canonical_bytes(m));
    hasher.finalize()
}

#[must_use]
pub fn mac(key: &cu_crypto::Secret<32>, m: &ManifestV2) -> [u8; 32] {
    cu_crypto::mac::hmac_sha256(key.expose(), &canonical_bytes(m))
}

#[must_use]
pub fn verify_mac(key: &cu_crypto::Secret<32>, m: &ManifestV2, candidate_mac: &[u8]) -> bool {
    cu_crypto::mac::verify(key.expose(), &canonical_bytes(m), candidate_mac)
}

#[must_use]
pub fn encode(m: &ManifestV2, mac_bytes: &[u8; 32]) -> Vec<u8> {
    #[derive(Serialize)]
    struct WithMac {
        #[serde(flatten)]
        manifest: WireManifest,
        mac: String,
    }
    let with_mac = WithMac {
        manifest: to_wire(m),
        mac: hex_encode(mac_bytes),
    };
    #[allow(clippy::expect_used)]
    let out = serde_json::to_vec(&with_mac).expect("manifest_v2 is always serializable");
    out
}

#[must_use]
pub fn peek_version(bytes: &[u8]) -> Option<u32> {
    #[derive(Deserialize)]
    struct Peek {
        #[serde(rename = "cryptoUploadManifestVersion")]
        version: u32,
    }
    serde_json::from_slice::<Peek>(bytes)
        .ok()
        .map(|p| p.version)
}

pub fn decode(bytes: &[u8]) -> Result<(ManifestV2, [u8; 32]), FormatError> {
    #[derive(Deserialize)]
    struct WithMac {
        #[serde(flatten)]
        manifest: WireManifest,
        mac: String,
    }
    let with_mac: WithMac = serde_json::from_slice(bytes)
        .map_err(|_| FormatError::Malformed("manifest v2 is not valid JSON"))?;
    let w = with_mac.manifest;
    if w.version != MANIFEST_V2_VERSION {
        return Err(FormatError::UnsupportedVersion(
            u8::try_from(w.version).unwrap_or(u8::MAX),
        ));
    }
    let mac_bytes = hex_decode_n::<32>(&with_mac.mac)
        .ok_or(FormatError::Malformed("manifest v2 mac is not 32-byte hex"))?;
    let prev = hex_decode_n::<32>(&w.prev).ok_or(FormatError::Malformed(
        "manifest v2 prev is not 32-byte hex",
    ))?;
    if hex_decode_n::<16>(&w.last_writer.device_id).is_none() {
        return Err(FormatError::Malformed(
            "lastWriter deviceId is not 16-byte hex",
        ));
    }
    let body = match &w.body {
        WireBody::Checkpoint { coverage, entries } => {
            let mut out = Vec::with_capacity(entries.len());
            for e in entries {
                out.push(CheckpointEntry {
                    key: key_from_wire(&e.key)?,
                    slot: Slot {
                        current: opt_node_from_wire(e.c.as_ref())?,
                        pending: e
                            .w
                            .iter()
                            .map(|p| opt_node_from_wire(p.as_ref()))
                            .collect::<Result<Vec<_>, _>>()?,
                    },
                });
            }
            Body::Checkpoint {
                coverage: *coverage,
                entries: out,
            }
        }
        WireBody::Delta { ops } => Body::Delta {
            ops: ops
                .iter()
                .map(op_from_wire)
                .collect::<Result<Vec<_>, _>>()?,
        },
    };
    Ok((
        ManifestV2 {
            seq: w.seq,
            updated_at_ms: w.updated_at_ms,
            last_writer: Writer {
                device_id: w.last_writer.device_id,
                label: w.last_writer.label,
            },
            prev,
            cp: w.cp,
            body,
        },
        mac_bytes,
    ))
}
