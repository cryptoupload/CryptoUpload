// SPDX-License-Identifier: GPL-3.0-or-later

use cu_index::NodeId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedMkdir {
    pub node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub name: String,
    pub vault_path: String,
    pub scan: cu_index::ScanFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedUpload {
    pub node_id: NodeId,
    pub parent_id: Option<NodeId>,
    pub name: String,
    pub vault_path: String,
    pub local_path: std::path::PathBuf,
    pub size: u64,
    pub mtime_ns: i64,
    pub scan: cu_index::ScanFingerprint,
    pub content_hash: [u8; 32],
    pub archive_previous: Option<(i64, String)>,
    pub skip_archive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRename {
    pub node_id: NodeId,
    pub old_parent_id: Option<NodeId>,
    pub old_name: String,
    pub old_vault_path: String,
    pub new_parent_id: Option<NodeId>,
    pub new_name: String,
    pub new_vault_path: String,
    pub kind: cu_index::NodeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDelete {
    pub node_id: NodeId,
    pub vault_path: String,
    pub kind: cu_index::NodeKind,
    pub archive_previous: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    ModifiedBothSides,
    VaultVanished,
    DeleteVsModify,
    NewFileCollision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    KeptBoth { sidecar_path: String },
    Resurrected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub node_id: NodeId,
    pub vault_path: String,
    pub reason: String,
    pub kind: ConflictKind,
    pub resolution: ConflictResolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSetAside {
    pub node_id: NodeId,
    pub vault_path: String,
    pub sidecar_path: String,
    pub forget_node: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub set_asides: Vec<PlannedSetAside>,
    pub mkdirs: Vec<PlannedMkdir>,
    pub uploads: Vec<PlannedUpload>,
    pub renames: Vec<PlannedRename>,
    pub deletes: Vec<PlannedDelete>,
    pub conflicts: Vec<Conflict>,
    pub skipped: Vec<String>,
    pub ignored_kept: Vec<String>,
    pub vault_prefix: String,
}

impl Plan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set_asides.is_empty()
            && self.mkdirs.is_empty()
            && self.uploads.is_empty()
            && self.renames.is_empty()
            && self.deletes.is_empty()
    }
}
