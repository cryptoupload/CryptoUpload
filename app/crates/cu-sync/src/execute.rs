// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use cu_backend::BackendError;
use cu_format::config::VaultConfig;
use cu_index::{Db, JournalOp, JournalState, NodeId, NodeRow, RootId, VersionLocation, VersionRow};
use cu_vfs::{DirRef, HeaderNonce, VaultFs, VfsError};

use crate::error::SyncError;
use crate::model::{
    Conflict, ConflictKind, ConflictResolution, Plan, PlannedDelete, PlannedMkdir, PlannedRename,
    PlannedSetAside, PlannedUpload,
};
use crate::progress::{CancelToken, SyncOp, SyncProgress};

const DB_TRANSACTION_CHUNK: usize = 200;

pub const DEFAULT_UPLOAD_CONCURRENCY: usize = 6;

const CONCURRENT_UPLOAD_MAX_BYTES: u64 = 128 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct ExecuteOptions {
    pub upload_concurrency: usize,
    pub shuffle_uploads: bool,
}

impl Default for ExecuteOptions {
    fn default() -> Self {
        Self {
            shuffle_uploads: false,
            upload_concurrency: DEFAULT_UPLOAD_CONCURRENCY,
        }
    }
}

fn shuffle<T>(items: &mut [T]) {
    for i in (1..items.len()).rev() {
        let mut bytes = [0u8; 8];
        if cu_crypto::random::fill_random(&mut bytes).is_err() {
            return;
        }
        let j = usize::try_from(u64::from_le_bytes(bytes) % (i as u64 + 1)).unwrap_or(0);
        items.swap(i, j);
    }
}

struct DirCache {
    enabled: bool,
    dirs: HashMap<String, DirRef>,
}

impl DirCache {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            dirs: HashMap::new(),
        }
    }

    async fn parent_of<'p>(
        &mut self,
        vault_fs: &VaultFs,
        config: &VaultConfig,
        vault_path: &'p str,
    ) -> Result<(DirRef, &'p str), VfsError> {
        let (parent, name) = vault_path
            .trim_matches('/')
            .rsplit_once('/')
            .unwrap_or(("", vault_path.trim_matches('/')));
        if let Some(dir) = self.dirs.get(parent) {
            return Ok((dir.clone(), name));
        }
        let dir = vault_fs.resolve_or_create_dir_ref(config, parent).await?;
        self.dirs.insert(parent.to_string(), dir.clone());
        Ok((dir, name))
    }

    async fn validate(&self, vault_fs: &VaultFs, config: &VaultConfig) -> Result<(), SyncError> {
        let mut paths: Vec<&String> = self.dirs.keys().collect();
        paths.sort();
        for path in paths {
            match vault_fs.resolve_dir_ref(config, path).await {
                Ok(fresh) if fresh.same_dir(&self.dirs[path]) => {}
                Ok(_) | Err(VfsError::NotFound) => {
                    return Err(SyncError::DirectoryChanged(path.clone()));
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.dirs.clear();
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecutionStats {
    pub set_asides: u64,
    pub mkdirs: u64,
    pub uploads: u64,
    pub modifies: u64,
    pub renames: u64,
    pub deletes: u64,
    pub bytes_transferred: u64,
}

pub(crate) fn archive_vault_path(node_id: NodeId, version: i64) -> String {
    let hex = hex_node_id(node_id);
    format!(".cryptoupload/versions/{}/{hex}.{version}", &hex[..2])
}

pub(crate) fn archive_dir_and_path(node_id: NodeId, version: i64) -> (String, String) {
    let hex = hex_node_id(node_id);
    (
        format!(".cryptoupload/versions/{}", &hex[..2]),
        format!(".cryptoupload/versions/{}/{hex}.{version}", &hex[..2]),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub keep_versions: usize,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self { keep_versions: 5 }
    }
}

pub(crate) async fn prune_excess_versions(
    db: &mut Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    node_id: NodeId,
    retention: RetentionPolicy,
    retention_blocked: &mut Vec<String>,
) -> Result<(), SyncError> {
    let excess = db.versions_beyond_limit(node_id, retention.keep_versions)?;
    forget_archived_versions(db, vault_fs, config, &excess, retention_blocked).await?;
    Ok(())
}

pub(crate) async fn forget_archived_versions(
    db: &mut Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    versions: &[cu_index::VersionRow],
    retention_blocked: &mut Vec<String>,
) -> Result<u64, SyncError> {
    let mut forgotten = 0u64;
    for v in versions {
        let path = archive_vault_path(v.node_id, v.version);
        match vault_fs.remove_file(config, &path).await {
            Ok(()) | Err(VfsError::NotFound) => {}
            Err(VfsError::Backend(BackendError::RetentionActive(_))) => {
                retention_blocked.push(path);
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        db.delete_version(v.node_id, v.version)?;
        forgotten += 1;
    }
    Ok(forgotten)
}

fn hex_node_id(node_id: NodeId) -> String {
    crate::hexutil::encode(&node_id.0)
}

#[derive(Debug, Default)]
pub struct ExecutionOutcome {
    pub stats: ExecutionStats,
    pub conflicts: Vec<Conflict>,
    pub retention_blocked: Vec<String>,
    pub skipped: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_plan_tuned(
    db: &mut Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    root_id: RootId,
    run_id: i64,
    plan: &Plan,
    retention: RetentionPolicy,
    now_ms: impl FnMut() -> i64,
    on_progress: &mut (dyn FnMut(SyncProgress) + Send),
    device_label: Option<&str>,
    cancel: &CancelToken,
    options: ExecuteOptions,
) -> Result<ExecutionOutcome, SyncError> {
    vault_fs.begin_dirmeta_batch();
    let mut exec = Executor {
        db,
        vault_fs,
        config,
        root_id,
        run_id,
        plan,
        retention,
        now_ms,
        on_progress,
        device_label,
        cancel,
        seq: 0,
        index: 0,
        total: plan.set_asides.len()
            + plan.mkdirs.len()
            + plan.uploads.len()
            + plan.renames.len()
            + plan.deletes.len(),
        ops_in_tx: 0,
        out: ExecutionOutcome::default(),
        delete_done: vec![false; plan.deletes.len()],
        delete_index: plan
            .deletes
            .iter()
            .enumerate()
            .map(|(i, d)| (d.node_id, i))
            .collect(),
        pending_target: HashMap::new(),
        dir_cache: DirCache::new(!vault_fs.backend().capabilities().stale_path_cache),
        upload_concurrency: options.upload_concurrency.max(1),
        shuffle_uploads: options.shuffle_uploads,
        batch: Vec::new(),
    };
    (exec.on_progress)(SyncProgress::Plan {
        items: exec.total,
        upload_bytes: plan.uploads.iter().map(|u| u.size).sum(),
    });
    let result = match exec.db.begin_transaction() {
        Ok(()) => match exec.run().await {
            Ok(()) => match exec.settle_dirs().await {
                Ok(()) => exec.db.commit_transaction().map_err(SyncError::from),
                Err(e) => Err(rollback_after(exec.db, e)),
            },
            Err(SyncError::Cancelled) => match exec.settle_dirs().await {
                Ok(()) => match exec.db.commit_transaction() {
                    Ok(()) => Err(SyncError::Cancelled),
                    Err(e) => Err(rollback_after(exec.db, e.into())),
                },
                Err(e) => Err(rollback_after(exec.db, e)),
            },
            Err(e) => Err(rollback_after(exec.db, e)),
        },
        Err(e) => Err(e.into()),
    };
    vault_fs.end_dirmeta_batch(config).await;
    result.map(|()| exec.out)
}

fn rollback_after(db: &Db, err: SyncError) -> SyncError {
    match db.rollback_transaction() {
        Ok(()) => err,
        Err(rollback_err) => SyncError::Io(std::io::Error::other(format!(
            "{err}; rolling back the state.db transaction also failed: \
             {rollback_err}"
        ))),
    }
}

#[derive(Clone, Copy)]
enum Placement<'p> {
    Rename(&'p PlannedRename),
    Mkdir(&'p PlannedMkdir),
    Upload(&'p PlannedUpload),
}

impl Placement<'_> {
    fn node_id(self) -> NodeId {
        match self {
            Self::Rename(r) => r.node_id,
            Self::Mkdir(m) => m.node_id,
            Self::Upload(u) => u.node_id,
        }
    }

    fn target(&self) -> (Option<NodeId>, &str) {
        match self {
            Self::Rename(r) => (r.new_parent_id, r.new_name.as_str()),
            Self::Mkdir(m) => (m.parent_id, m.name.as_str()),
            Self::Upload(u) => (u.parent_id, u.name.as_str()),
        }
    }

    fn describe(&self) -> &str {
        match self {
            Self::Rename(r) => &r.new_vault_path,
            Self::Mkdir(m) => &m.vault_path,
            Self::Upload(u) => &u.vault_path,
        }
    }
}

enum Readiness {
    Ready,
    Wait { blocker: Option<NodeId> },
    NeedsDelete(NodeRow),
}

enum EarlyDelete {
    Done,
    BlockedBy(NodeId),
}

const MAX_PLACEMENT_ROUNDS: usize = 100_000;

struct BatchedUpload {
    u: PlannedUpload,
    seq: i64,
    file: tokio::fs::File,
    dir: DirRef,
    name: String,
}

struct Executor<'a, N: FnMut() -> i64> {
    db: &'a mut Db,
    vault_fs: &'a VaultFs,
    config: &'a VaultConfig,
    root_id: RootId,
    run_id: i64,
    plan: &'a Plan,
    retention: RetentionPolicy,
    now_ms: N,
    on_progress: &'a mut (dyn FnMut(SyncProgress) + Send),
    device_label: Option<&'a str>,
    cancel: &'a CancelToken,
    seq: i64,
    index: usize,
    total: usize,
    ops_in_tx: usize,
    out: ExecutionOutcome,
    delete_done: Vec<bool>,
    delete_index: HashMap<NodeId, usize>,
    pending_target: HashMap<NodeId, (Option<NodeId>, String)>,
    dir_cache: DirCache,
    upload_concurrency: usize,
    shuffle_uploads: bool,
    batch: Vec<BatchedUpload>,
}

impl<N: FnMut() -> i64> Executor<'_, N> {
    async fn run(&mut self) -> Result<(), SyncError> {
        for s in &self.plan.set_asides {
            self.progress(&s.vault_path, SyncOp::SetAside)?;
            self.settle_dirs().await?;
            run_set_aside(
                self.db,
                self.vault_fs,
                self.config,
                self.root_id,
                self.run_id,
                self.seq,
                s,
                &mut self.now_ms,
            )
            .await?;
            self.out.stats.set_asides += 1;
            self.tick().await?;
        }
        self.run_placements().await?;
        for i in 0..self.plan.deletes.len() {
            if !self.delete_done[i] {
                self.run_delete_at(i, false).await?;
            }
        }
        Ok(())
    }

    async fn tick(&mut self) -> Result<(), SyncError> {
        self.ops_in_tx += 1;
        if self.ops_in_tx >= DB_TRANSACTION_CHUNK {
            self.settle_dirs().await?;
            self.db.commit_transaction()?;
            self.db.begin_transaction()?;
            self.ops_in_tx = 0;
        }
        Ok(())
    }

    async fn commit_now(&mut self) -> Result<(), SyncError> {
        self.settle_dirs().await?;
        self.db.commit_transaction()?;
        self.db.begin_transaction()?;
        self.ops_in_tx = 0;
        Ok(())
    }

    async fn settle_dirs(&mut self) -> Result<(), SyncError> {
        if self.dir_cache.enabled {
            self.dir_cache.validate(self.vault_fs, self.config).await?;
        }
        self.dir_cache.clear();
        Ok(())
    }

    fn progress(&mut self, vault_path: &str, op: SyncOp) -> Result<(), SyncError> {
        if self.cancel.is_cancelled() {
            return Err(SyncError::Cancelled);
        }
        self.seq += 1;
        self.index += 1;
        (self.on_progress)(SyncProgress::Item {
            vault_path: vault_path.to_string(),
            op,
            index: self.index,
            total: self.total,
        });
        Ok(())
    }

    fn path_of(&self, node_id: NodeId) -> Result<Option<String>, SyncError> {
        crate::restore::logical_path_of(self.db, &self.plan.vault_prefix, node_id)
    }

    fn child_path(
        &self,
        parent_id: Option<NodeId>,
        name: &str,
    ) -> Result<Option<String>, SyncError> {
        let parent = match parent_id {
            None => self.plan.vault_prefix.clone(),
            Some(p) => match self.path_of(p)? {
                Some(path) => path,
                None => return Ok(None),
            },
        };
        Ok(Some(if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}/{name}")
        }))
    }

    fn moves_away(&self, node: &NodeRow) -> bool {
        self.pending_target
            .get(&node.node_id)
            .is_some_and(|(parent, name)| *parent != node.parent_id || *name != node.name)
    }

    fn pending_delete_of(&self, node_id: NodeId) -> Option<usize> {
        self.delete_index
            .get(&node_id)
            .copied()
            .filter(|&i| !self.delete_done[i])
    }

    fn is_self_or_descendant(
        &self,
        candidate: NodeId,
        ancestor: NodeId,
    ) -> Result<bool, SyncError> {
        let mut current = Some(candidate);
        let mut depth = 0usize;
        while let Some(id) = current {
            if id == ancestor {
                return Ok(true);
            }
            depth += 1;
            if depth > 4096 {
                return Err(SyncError::IntegrityFailure(cu_msg::Msg::new(
                    "SYNC_STATE_CYCLE",
                )));
            }
            current = self.db.get_node(id)?.and_then(|n| n.parent_id);
        }
        Ok(false)
    }

    fn readiness(&self, op: Placement<'_>) -> Result<Readiness, SyncError> {
        let (parent_id, name) = op.target();
        if let Some(parent) = parent_id {
            if self.db.get_node(parent)?.is_none() {
                return Ok(Readiness::Wait { blocker: None });
            }
            if self.is_self_or_descendant(parent, op.node_id())? {
                return Ok(Readiness::Wait { blocker: None });
            }
        }
        let Some(occupant) = self.db.find_child(self.root_id, parent_id, name)? else {
            return Ok(Readiness::Ready);
        };
        if occupant.node_id == op.node_id() {
            return Ok(Readiness::Ready);
        }
        if self.moves_away(&occupant) {
            return Ok(Readiness::Wait {
                blocker: Some(occupant.node_id),
            });
        }
        if self.pending_delete_of(occupant.node_id).is_some() {
            return Ok(Readiness::NeedsDelete(occupant));
        }
        Ok(Readiness::Wait { blocker: None })
    }

    async fn run_placements(&mut self) -> Result<(), SyncError> {
        let plan = self.plan;
        let mut pending: Vec<Option<Placement<'_>>> = plan
            .renames
            .iter()
            .map(Placement::Rename)
            .chain(plan.mkdirs.iter().map(Placement::Mkdir))
            .chain(plan.uploads.iter().map(Placement::Upload))
            .map(Some)
            .collect();
        if self.shuffle_uploads {
            let first_upload = plan.renames.len() + plan.mkdirs.len();
            if let Some(uploads) = pending.get_mut(first_upload..) {
                shuffle(uploads);
            }
        }
        for op in pending.iter().flatten() {
            let (parent, name) = op.target();
            self.pending_target
                .insert(op.node_id(), (parent, name.to_string()));
        }
        let mut remaining = pending.len();
        let mut rounds = 0usize;
        while remaining > 0 && rounds < MAX_PLACEMENT_ROUNDS {
            rounds += 1;
            let mut progressed = false;
            let mut park_candidate: Option<(NodeId, Option<NodeId>)> = None;
            for slot in &mut pending {
                let Some(op) = *slot else { continue };
                match self.readiness(op)? {
                    Readiness::Ready => {
                        *slot = None;
                        remaining -= 1;
                        self.pending_target.remove(&op.node_id());
                        if self.is_batchable(op) {
                            self.queue_upload(op).await?;
                        } else {
                            self.flush_batch().await?;
                            self.execute_placement(op).await?;
                        }
                        progressed = true;
                    }
                    Readiness::NeedsDelete(occupant) => {
                        self.flush_batch().await?;
                        match self.delete_early(occupant.node_id).await? {
                            EarlyDelete::Done => progressed = true,
                            EarlyDelete::BlockedBy(child) => {
                                park_candidate.get_or_insert((child, occupant.parent_id));
                            }
                        }
                    }
                    Readiness::Wait { blocker } => {
                        if let Some(blocker) = blocker
                            && let Some(row) = self.db.get_node(blocker)?
                        {
                            park_candidate.get_or_insert((blocker, row.parent_id));
                        }
                    }
                }
            }
            self.flush_batch().await?;
            if progressed {
                continue;
            }
            match park_candidate {
                Some((node_id, into)) => self.park(node_id, into).await?,
                None => break,
            }
        }
        for op in pending.into_iter().flatten() {
            self.pending_target.remove(&op.node_id());
            self.out.skipped.push(
                cu_msg::Msg::new("SYNC_SKIP_DESTINATION_BUSY")
                    .arg(op.describe())
                    .wire(),
            );
        }
        Ok(())
    }

    async fn execute_placement(&mut self, op: Placement<'_>) -> Result<(), SyncError> {
        match op {
            Placement::Rename(r) => {
                self.progress(&r.new_vault_path, SyncOp::Rename)?;
                let (Some(old), Some(new)) = (
                    self.path_of(r.node_id)?,
                    self.child_path(r.new_parent_id, &r.new_name)?,
                ) else {
                    return Ok(());
                };
                let resolved = PlannedRename {
                    old_vault_path: old,
                    new_vault_path: new,
                    ..r.clone()
                };
                self.settle_dirs().await?;
                let conflict = run_rename(
                    self.db,
                    self.vault_fs,
                    self.config,
                    self.root_id,
                    self.run_id,
                    self.seq,
                    &resolved,
                    &mut self.now_ms,
                    self.device_label,
                )
                .await?;
                self.out.conflicts.extend(conflict);
                self.out.stats.renames += 1;
                return self.commit_now().await;
            }
            Placement::Mkdir(m) => {
                self.progress(&m.vault_path, SyncOp::Mkdir)?;
                let Some(path) = self.child_path(m.parent_id, &m.name)? else {
                    return Ok(());
                };
                let resolved = PlannedMkdir {
                    vault_path: path,
                    ..m.clone()
                };
                run_mkdir(
                    self.db,
                    self.vault_fs,
                    self.config,
                    self.root_id,
                    self.run_id,
                    self.seq,
                    &resolved,
                    &mut self.now_ms,
                )
                .await?;
                self.out.stats.mkdirs += 1;
            }
            Placement::Upload(u) => self.execute_upload_placement(u).await?,
        }
        self.tick().await
    }

    fn is_batchable(&self, op: Placement<'_>) -> bool {
        let Placement::Upload(u) = op else {
            return false;
        };
        self.upload_concurrency > 1
            && self.dir_cache.enabled
            && u.archive_previous.is_none()
            && !u.skip_archive
            && u.size <= CONCURRENT_UPLOAD_MAX_BYTES
            && self.vault_fs.backend().capabilities().conditional_create
    }

    async fn queue_upload(&mut self, op: Placement<'_>) -> Result<(), SyncError> {
        let Placement::Upload(u) = op else {
            return Ok(());
        };
        if let Err(e) = self.progress(&u.vault_path, SyncOp::Upload) {
            self.flush_batch().await?;
            return Err(e);
        }
        let Some(target) = self.child_path(u.parent_id, &u.name)? else {
            return Ok(());
        };
        let resolved = PlannedUpload {
            vault_path: target,
            ..u.clone()
        };
        let file = match tokio::fs::File::open(&resolved.local_path).await {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                self.out.skipped.push(
                    cu_msg::Msg::new("SYNC_SKIP_READ_DENIED_AT_UPLOAD")
                        .arg(&resolved.vault_path)
                        .wire(),
                );
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let (dir, name) = self
            .dir_cache
            .parent_of(self.vault_fs, self.config, &resolved.vault_path)
            .await?;
        let name = name.to_string();
        self.batch.push(BatchedUpload {
            u: resolved,
            seq: self.seq,
            file,
            dir,
            name,
        });
        if self.batch.len() >= self.upload_concurrency {
            self.flush_batch().await?;
        }
        Ok(())
    }

    async fn flush_batch(&mut self) -> Result<(), SyncError> {
        use futures_util::StreamExt as _;

        let items = std::mem::take(&mut self.batch);
        if items.is_empty() {
            return Ok(());
        }
        let vault_fs = self.vault_fs;
        let config = self.config;
        let width = self.upload_concurrency;
        let mut results: Vec<(usize, BatchedUpload, Result<HeaderNonce, VfsError>)> =
            futures_util::stream::iter(items.into_iter().enumerate().map(
                |(index, mut item)| async move {
                    let outcome = vault_fs
                        .write_file_if_absent_with_nonce_in(
                            config,
                            &item.dir,
                            &item.name,
                            &mut item.file,
                            Some(item.u.content_hash),
                            &mut |_| {},
                        )
                        .await;
                    (index, item, outcome)
                },
            ))
            .buffer_unordered(width)
            .collect()
            .await;
        results.sort_by_key(|(index, _, _)| *index);

        for (_, item, outcome) in results {
            let BatchedUpload { u, seq, .. } = item;
            match outcome {
                Ok(nonce) => {
                    self.record_journal(u.node_id, seq)?;
                    let upload_ts = (self.now_ms)();
                    commit_upload(self.db, self.root_id, &u, upload_ts, Some(nonce))?;
                    self.out.stats.uploads += 1;
                    self.out.stats.bytes_transferred += u.size;
                }
                Err(_) => self.run_resolved_upload(&u).await?,
            }
            self.tick().await?;
        }
        Ok(())
    }

    fn record_journal(&mut self, node_id: NodeId, seq: i64) -> Result<(), SyncError> {
        let entry_id = self.db.journal_plan(
            self.run_id,
            self.root_id,
            node_id,
            seq,
            JournalOp::Upload,
            "{}",
            (self.now_ms)(),
        )?;
        for state in [
            JournalState::Started,
            JournalState::RemoteDone,
            JournalState::Committed,
        ] {
            self.db.journal_advance(entry_id, state, (self.now_ms)())?;
        }
        Ok(())
    }

    async fn execute_upload_placement(&mut self, u: &PlannedUpload) -> Result<(), SyncError> {
        self.progress(&u.vault_path, SyncOp::Upload)?;
        let Some(target) = self.child_path(u.parent_id, &u.name)? else {
            return Ok(());
        };
        let archive_previous = match &u.archive_previous {
            Some((version, planned_old)) => Some((
                *version,
                self.path_of(u.node_id)?
                    .unwrap_or_else(|| planned_old.clone()),
            )),
            None => None,
        };
        let resolved = PlannedUpload {
            vault_path: target,
            archive_previous,
            ..u.clone()
        };
        self.run_resolved_upload(&resolved).await
    }

    async fn run_resolved_upload(&mut self, resolved: &PlannedUpload) -> Result<(), SyncError> {
        let u = resolved;
        match run_upload(
            self.db,
            self.vault_fs,
            self.config,
            self.root_id,
            self.run_id,
            self.seq,
            resolved,
            self.retention,
            &mut self.now_ms,
            &mut *self.on_progress,
            self.device_label,
            &mut self.out.retention_blocked,
            &mut self.dir_cache,
        )
        .await?
        {
            UploadOutcome::Written(conflict) => {
                self.out.conflicts.extend(conflict);
                self.out.stats.uploads += 1;
                self.out.stats.modifies += u64::from(u.archive_previous.is_some());
                self.out.stats.bytes_transferred += u.size;
            }
            UploadOutcome::AlreadyThere(conflict) => {
                self.out.conflicts.extend(conflict);
                self.out.stats.uploads += 1;
                self.out.stats.modifies += u64::from(u.archive_previous.is_some());
            }
            UploadOutcome::Skipped(reason) => {
                if let Some(reason) = reason {
                    self.out.skipped.push(reason);
                }
            }
        }
        Ok(())
    }

    async fn delete_early(&mut self, node_id: NodeId) -> Result<EarlyDelete, SyncError> {
        let Some(root_delete) = self.pending_delete_of(node_id) else {
            return Ok(EarlyDelete::Done);
        };
        let mut subtree = Vec::new();
        let mut stack = vec![node_id];
        while let Some(id) = stack.pop() {
            for child in self.db.list_children(self.root_id, Some(id))? {
                if self.pending_target.contains_key(&child.node_id) {
                    return Ok(EarlyDelete::BlockedBy(child.node_id));
                }
                stack.push(child.node_id);
                subtree.push(child.node_id);
            }
        }
        for id in subtree {
            if let Some(i) = self.pending_delete_of(id) {
                self.run_delete_at(i, true).await?;
            }
        }
        self.run_delete_at(root_delete, true).await?;
        Ok(EarlyDelete::Done)
    }

    async fn run_delete_at(&mut self, i: usize, early: bool) -> Result<(), SyncError> {
        self.delete_done[i] = true;
        let d = &self.plan.deletes[i];
        self.progress(&d.vault_path, SyncOp::Delete)?;
        let Some(path) = self.path_of(d.node_id)? else {
            return Ok(());
        };
        let resolved = PlannedDelete {
            vault_path: path,
            ..d.clone()
        };
        if resolved.kind == cu_index::NodeKind::Dir {
            self.settle_dirs().await?;
        }
        match run_delete(
            self.db,
            self.vault_fs,
            self.config,
            self.root_id,
            self.run_id,
            self.seq,
            &resolved,
            self.retention,
            &mut self.now_ms,
            &mut self.out.retention_blocked,
            early,
            self.device_label,
        )
        .await?
        {
            None => self.out.stats.deletes += 1,
            Some(conflict) => self.out.conflicts.push(conflict),
        }
        if early {
            self.commit_now().await
        } else {
            self.tick().await
        }
    }

    async fn park(&mut self, node_id: NodeId, into: Option<NodeId>) -> Result<(), SyncError> {
        let Some(node) = self.db.get_node(node_id)? else {
            return Ok(());
        };
        let hex = hex_node_id(node_id);
        let mut temp_name = format!("{}.cu-{}", node.name, &hex[..8]);
        if self
            .db
            .find_child(self.root_id, into, &temp_name)?
            .is_some()
        {
            temp_name = format!("{}.cu-{hex}", node.name);
        }
        let (Some(old), Some(new)) = (self.path_of(node_id)?, self.child_path(into, &temp_name)?)
        else {
            return Ok(());
        };
        let parked = PlannedRename {
            node_id,
            old_parent_id: node.parent_id,
            old_name: node.name.clone(),
            old_vault_path: old,
            new_parent_id: into,
            new_name: temp_name.clone(),
            new_vault_path: new,
            kind: node.kind,
        };
        self.seq += 1;
        self.settle_dirs().await?;
        let vault_fs = self.vault_fs;
        let config = self.config;
        run_journaled(
            self.db,
            self.run_id,
            self.root_id,
            node_id,
            self.seq,
            JournalOp::Rename,
            &mut self.now_ms,
            || execute_rename(vault_fs, config, &parked, None, 0),
        )
        .await?;
        self.db.rename_node(node_id, into, &temp_name)?;
        self.commit_now().await
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_mkdir(
    db: &mut Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    root_id: RootId,
    run_id: i64,
    seq: i64,
    m: &PlannedMkdir,
    now_ms: &mut impl FnMut() -> i64,
) -> Result<(), SyncError> {
    run_journaled(
        db,
        run_id,
        root_id,
        m.node_id,
        seq,
        JournalOp::Mkdir,
        now_ms,
        || execute_mkdir(vault_fs, config, m),
    )
    .await?;
    insert_node_idempotent(
        db,
        &NodeRow {
            node_id: m.node_id,
            root_id,
            parent_id: m.parent_id,
            name: m.name.clone(),
            kind: cu_index::NodeKind::Dir,
            size: 0,
            mtime_ns: m.scan.mtime_ns,
            content_hash: None,
            version: 0,
            scan: Some(m.scan),
            header_nonce: None,
        },
    )?;
    Ok(())
}

enum UploadOutcome {
    Written(Option<Conflict>),
    AlreadyThere(Option<Conflict>),
    Skipped(Option<String>),
}

#[allow(clippy::too_many_arguments)]
async fn run_upload(
    db: &mut Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    root_id: RootId,
    run_id: i64,
    seq: i64,
    u: &PlannedUpload,
    retention: RetentionPolicy,
    now_ms: &mut impl FnMut() -> i64,
    on_progress: &mut (dyn FnMut(SyncProgress) + Send),
    device_label: Option<&str>,
    retention_blocked: &mut Vec<String>,
    dir_cache: &mut DirCache,
) -> Result<UploadOutcome, SyncError> {
    let file = match tokio::fs::File::open(&u.local_path).await {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UploadOutcome::Skipped(None));
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Ok(UploadOutcome::Skipped(Some(
                cu_msg::Msg::new("SYNC_SKIP_READ_DENIED_AT_UPLOAD")
                    .arg(&u.vault_path)
                    .wire(),
            )));
        }
        Err(e) => return Err(e.into()),
    };
    let upload_ts = now_ms();
    let baseline_nonce = baseline_nonce_to_archive(db, u)?;
    let (written, conflict, header_nonce) = run_journaled(
        db,
        run_id,
        root_id,
        u.node_id,
        seq,
        JournalOp::Upload,
        now_ms,
        || {
            execute_upload(
                vault_fs,
                config,
                u,
                file,
                on_progress,
                device_label,
                upload_ts,
                dir_cache,
                baseline_nonce,
            )
        },
    )
    .await?;
    commit_upload(db, root_id, u, now_ms(), header_nonce)?;
    if u.archive_previous.is_some() {
        prune_excess_versions(
            db,
            vault_fs,
            config,
            u.node_id,
            retention,
            retention_blocked,
        )
        .await?;
    }
    Ok(if written {
        UploadOutcome::Written(conflict)
    } else {
        UploadOutcome::AlreadyThere(conflict)
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_set_aside(
    db: &mut Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    root_id: RootId,
    run_id: i64,
    seq: i64,
    s: &PlannedSetAside,
    now_ms: &mut impl FnMut() -> i64,
) -> Result<(), SyncError> {
    run_journaled(
        db,
        run_id,
        root_id,
        s.node_id,
        seq,
        JournalOp::SetAside,
        now_ms,
        || execute_set_aside(vault_fs, config, s),
    )
    .await?;
    if s.forget_node {
        db.delete_node(s.node_id)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_rename(
    db: &mut Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    root_id: RootId,
    run_id: i64,
    seq: i64,
    r: &PlannedRename,
    now_ms: &mut impl FnMut() -> i64,
    device_label: Option<&str>,
) -> Result<Option<Conflict>, SyncError> {
    let conflict_ts = now_ms();
    let conflict = run_journaled(
        db,
        run_id,
        root_id,
        r.node_id,
        seq,
        JournalOp::Rename,
        now_ms,
        || execute_rename(vault_fs, config, r, device_label, conflict_ts),
    )
    .await?;
    db.rename_node(r.node_id, r.new_parent_id, &r.new_name)?;
    if r.kind == cu_index::NodeKind::File
        && let Some(node) = db.get_node(r.node_id)?
    {
        let new_version = node.version + 1;
        insert_version_idempotent(
            db,
            &VersionRow {
                node_id: r.node_id,
                version: new_version,
                created_at: now_ms(),
                kind: cu_index::NodeKind::File,
                parent_id: r.new_parent_id,
                name: r.new_name.clone(),
                size: node.size,
                content_hash: node.content_hash,
                location: VersionLocation::Canonical,
                deleted: false,
            },
        )?;
        db.update_baseline(
            r.node_id,
            node.size,
            node.mtime_ns,
            node.content_hash,
            new_version,
            node.scan,
            node.header_nonce,
        )?;
    }
    Ok(conflict)
}

#[allow(clippy::too_many_arguments)]
async fn run_delete(
    db: &mut Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    root_id: RootId,
    run_id: i64,
    seq: i64,
    d: &PlannedDelete,
    retention: RetentionPolicy,
    now_ms: &mut impl FnMut() -> i64,
    retention_blocked: &mut Vec<String>,
    must_free_path: bool,
    device_label: Option<&str>,
) -> Result<Option<Conflict>, SyncError> {
    let conflict_ts = now_ms();
    let kept = run_journaled(
        db,
        run_id,
        root_id,
        d.node_id,
        seq,
        JournalOp::Delete,
        now_ms,
        || {
            execute_delete(
                vault_fs,
                config,
                d,
                must_free_path,
                device_label,
                conflict_ts,
            )
        },
    )
    .await?;
    if let (Some(version), cu_index::NodeKind::File) = (d.archive_previous, d.kind) {
        db.set_version_location(d.node_id, version, VersionLocation::Archived)?;
        insert_version_idempotent(
            db,
            &VersionRow {
                node_id: d.node_id,
                version: version + 1,
                created_at: now_ms(),
                kind: cu_index::NodeKind::File,
                parent_id: None,
                name: String::new(),
                size: 0,
                content_hash: None,
                location: VersionLocation::Archived,
                deleted: true,
            },
        )?;
        prune_excess_versions(
            db,
            vault_fs,
            config,
            d.node_id,
            retention,
            retention_blocked,
        )
        .await?;
    }
    db.delete_node(d.node_id)?;
    Ok(kept)
}

fn insert_node_idempotent(db: &Db, row: &NodeRow) -> Result<(), SyncError> {
    match db.insert_node(row) {
        Ok(()) => Ok(()),
        Err(e) if is_constraint_violation(&e) && db.get_node(row.node_id)?.is_some() => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn insert_version_idempotent(db: &Db, row: &VersionRow) -> Result<(), SyncError> {
    match db.insert_version(row) {
        Ok(()) => Ok(()),
        Err(e)
            if is_constraint_violation(&e)
                && db
                    .list_versions(row.node_id)?
                    .iter()
                    .any(|v| v.version == row.version) =>
        {
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn is_constraint_violation(err: &cu_index::IndexError) -> bool {
    matches!(
        err,
        cu_index::IndexError::Sqlite(e)
            if e.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation)
    )
}

#[allow(clippy::too_many_arguments)]
async fn run_journaled<F, Fut, T>(
    db: &mut Db,
    run_id: i64,
    root_id: RootId,
    node_id: NodeId,
    seq: i64,
    op: JournalOp,
    now_ms: &mut impl FnMut() -> i64,
    action: F,
) -> Result<T, SyncError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, SyncError>>,
{
    let entry_id = db.journal_plan(run_id, root_id, node_id, seq, op, "{}", now_ms())?;
    db.journal_advance(entry_id, JournalState::Started, now_ms())?;
    let result = action().await?;
    db.journal_advance(entry_id, JournalState::RemoteDone, now_ms())?;
    db.journal_advance(entry_id, JournalState::Committed, now_ms())?;
    Ok(result)
}

async fn execute_mkdir(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    m: &PlannedMkdir,
) -> Result<(), SyncError> {
    vault_fs.mkdir(config, &m.vault_path).await?;
    Ok(())
}

enum Occupant {
    Identical(HeaderNonce),
    SetAside(Conflict),
    Vanished,
}

async fn set_aside_if_different(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    u: &PlannedUpload,
    device_label: Option<&str>,
    now_ms: i64,
) -> Result<Occupant, SyncError> {
    let mut sink = crate::verify::HashingSink(cu_crypto::hash::Hasher::new());
    match vault_fs
        .read_file_with_nonce(config, &u.vault_path, &mut sink, &mut |_| {})
        .await
    {
        Ok(nonce) => {
            let actual = sink.0.finalize();
            if actual == u.content_hash {
                return Ok(Occupant::Identical(nonce));
            }
            let sidecar = crate::conflict::sidecar_path(
                vault_fs,
                config,
                &u.vault_path,
                device_label,
                now_ms,
            )
            .await?;
            match vault_fs.rename(config, &u.vault_path, &sidecar).await {
                Ok(()) | Err(VfsError::NotFound) => {}
                Err(e) => return Err(e.into()),
            }
            Ok(Occupant::SetAside(Conflict {
                node_id: u.node_id,
                vault_path: u.vault_path.clone(),
                reason: cu_msg::Msg::new("CONFLICT_SAME_INSTANT").wire(),
                kind: ConflictKind::NewFileCollision,
                resolution: ConflictResolution::KeptBoth {
                    sidecar_path: sidecar,
                },
            }))
        }
        Err(VfsError::NotFound) => Ok(Occupant::Vanished),
        Err(e) => Err(e.into()),
    }
}

const MAX_EXCLUSIVE_ATTEMPTS: u32 = 5;

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn execute_upload(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    u: &PlannedUpload,
    mut file: tokio::fs::File,
    on_progress: &mut (dyn FnMut(SyncProgress) + Send),
    device_label: Option<&str>,
    now_ms: i64,
    dir_cache: &mut DirCache,
    baseline_nonce: Option<HeaderNonce>,
) -> Result<(bool, Option<Conflict>, Option<HeaderNonce>), SyncError> {
    use tokio::io::AsyncSeekExt as _;

    let mut conflict = None;
    let exclusive_supported = vault_fs.backend().capabilities().conditional_create;

    if !exclusive_supported
        && u.archive_previous.is_none()
        && vault_fs.exists(config, &u.vault_path).await?
        && let Occupant::SetAside(c) =
            set_aside_if_different(vault_fs, config, u, device_label, now_ms).await?
    {
        conflict = Some(c);
    }

    let mut already_there: Option<HeaderNonce> = None;
    if let Some((old_version, old_path)) = &u.archive_previous
        && !u.skip_archive
    {
        match resolve_archive_state(vault_fs, config, u, *old_version, old_path, baseline_nonce)
            .await?
        {
            ArchiveState::NeedsArchiving => {
                let (versions_dir, archive_path) = archive_dir_and_path(u.node_id, *old_version);
                vault_fs.mkdir(config, &versions_dir).await?;
                match vault_fs.rename(config, old_path, &archive_path).await {
                    Ok(()) | Err(VfsError::NotFound) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            ArchiveState::AlreadyHandled => {}
            ArchiveState::NoOpAlreadyWritten(nonce) => {
                already_there = Some(nonce);
            }
            ArchiveState::Drifted(reason) => {
                conflict = Some(Conflict {
                    node_id: u.node_id,
                    vault_path: old_path.clone(),
                    reason,
                    kind: ConflictKind::VaultVanished,
                    resolution: ConflictResolution::Resurrected,
                });
            }
        }
    }

    if let Some(nonce) = already_there {
        return Ok((false, conflict, Some(nonce)));
    }

    if exclusive_supported {
        for attempt in 1..=MAX_EXCLUSIVE_ATTEMPTS {
            if attempt > 1 {
                file.rewind().await?;
            }
            match write_upload(vault_fs, config, dir_cache, u, &mut file, true, on_progress).await {
                Ok(nonce) => return Ok((true, conflict, Some(nonce))),
                Err(VfsError::AlreadyExists) => {
                    match set_aside_if_different(vault_fs, config, u, device_label, now_ms).await? {
                        Occupant::SetAside(c) => {
                            conflict = Some(c);
                        }
                        Occupant::Identical(nonce) => {
                            return Ok((false, conflict, Some(nonce)));
                        }
                        Occupant::Vanished => {}
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(SyncError::Vfs(VfsError::AlreadyExists))
    } else {
        let nonce = write_upload(
            vault_fs,
            config,
            dir_cache,
            u,
            &mut file,
            false,
            on_progress,
        )
        .await?;
        Ok((true, conflict, Some(nonce)))
    }
}

async fn write_upload(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    dir_cache: &mut DirCache,
    u: &PlannedUpload,
    file: &mut tokio::fs::File,
    exclusive: bool,
    on_progress: &mut (dyn FnMut(SyncProgress) + Send),
) -> Result<HeaderNonce, VfsError> {
    let mut on_bytes = |count| on_progress(SyncProgress::Bytes { count });
    if dir_cache.enabled {
        let (dir, name) = dir_cache.parent_of(vault_fs, config, &u.vault_path).await?;
        if exclusive {
            vault_fs
                .write_file_if_absent_with_nonce_in(
                    config,
                    &dir,
                    name,
                    file,
                    Some(u.content_hash),
                    &mut on_bytes,
                )
                .await
        } else {
            vault_fs
                .write_file_with_nonce_in(
                    config,
                    &dir,
                    name,
                    file,
                    Some(u.content_hash),
                    &mut on_bytes,
                )
                .await
        }
    } else if exclusive {
        vault_fs
            .write_file_if_absent_with_nonce(
                config,
                &u.vault_path,
                file,
                Some(u.content_hash),
                &mut on_bytes,
            )
            .await
    } else {
        vault_fs
            .write_file_with_nonce(
                config,
                &u.vault_path,
                file,
                Some(u.content_hash),
                &mut on_bytes,
            )
            .await
    }
}

enum ArchiveState {
    NeedsArchiving,
    AlreadyHandled,
    NoOpAlreadyWritten(HeaderNonce),
    Drifted(String),
}

fn baseline_nonce_to_archive(db: &Db, u: &PlannedUpload) -> Result<Option<HeaderNonce>, SyncError> {
    if u.archive_previous.is_none() || u.skip_archive {
        return Ok(None);
    }
    Ok(db
        .get_node(u.node_id)?
        .and_then(|node| match (node.content_hash, node.header_nonce) {
            (Some(hash), Some(nonce)) if hash != u.content_hash => Some(nonce),
            _ => None,
        }))
}

async fn resolve_archive_state(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    u: &PlannedUpload,
    old_version: i64,
    old_path: &str,
    baseline_nonce: Option<HeaderNonce>,
) -> Result<ArchiveState, SyncError> {
    let (node_id, vault_path, new_content_hash) =
        (u.node_id, u.vault_path.as_str(), u.content_hash);
    if let Some(nonce) = baseline_nonce
        && matches!(vault_fs.read_header_nonce(config, old_path).await, Ok(found) if found == nonce)
    {
        return Ok(ArchiveState::NeedsArchiving);
    }
    if vault_fs.exists(config, old_path).await? {
        let mut sink = crate::verify::HashingSink(cu_crypto::hash::Hasher::new());
        match vault_fs
            .read_file_with_nonce(config, old_path, &mut sink, &mut |_| {})
            .await
        {
            Ok(nonce) => {
                let actual = sink.0.finalize();
                if actual == new_content_hash {
                    if old_path == vault_path {
                        Ok(ArchiveState::NoOpAlreadyWritten(nonce))
                    } else {
                        Ok(ArchiveState::AlreadyHandled)
                    }
                } else {
                    Ok(ArchiveState::NeedsArchiving)
                }
            }
            Err(VfsError::NotFound) => Ok(ArchiveState::NeedsArchiving),
            Err(e) => Err(e.into()),
        }
    } else {
        let archive_path = archive_vault_path(node_id, old_version);
        if vault_fs.exists(config, &archive_path).await? {
            Ok(ArchiveState::AlreadyHandled)
        } else {
            Ok(ArchiveState::Drifted(
                cu_msg::Msg::new("CONFLICT_ARCHIVE_VANISHED")
                    .arg(old_path)
                    .wire(),
            ))
        }
    }
}

async fn execute_set_aside(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    s: &PlannedSetAside,
) -> Result<(), SyncError> {
    match copy_within_vault_exclusive(vault_fs, config, &s.vault_path, &s.sidecar_path).await {
        Ok(()) => {}
        Err(VfsError::NotFound) => return Ok(()),
        Err(VfsError::AlreadyExists) => {
            let dest = format!("{}.{}", s.sidecar_path, hex_node_id(s.node_id));
            match copy_within_vault_exclusive(vault_fs, config, &s.vault_path, &dest).await {
                Ok(()) | Err(VfsError::AlreadyExists) => {}
                Err(VfsError::NotFound) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
        Err(e) => return Err(e.into()),
    }
    match vault_fs.remove_file(config, &s.vault_path).await {
        Ok(()) | Err(VfsError::NotFound) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

async fn copy_within_vault_exclusive(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    src: &str,
    dest: &str,
) -> Result<(), VfsError> {
    use tokio::io::AsyncWriteExt as _;

    let (mut sink, mut source) = tokio::io::duplex(64 * 1024);
    let mut on_read_bytes = |_| {};
    let mut on_write_bytes = |_| {};
    let read_fut = async {
        let res = vault_fs
            .read_file(config, src, &mut sink, &mut on_read_bytes)
            .await;
        let _ = sink.shutdown().await;
        res
    };
    let write_fut =
        vault_fs.write_file_if_absent(config, dest, &mut source, None, &mut on_write_bytes);
    let (read_res, write_res) = tokio::join!(read_fut, write_fut);
    write_res?;
    read_res
}

async fn execute_rename(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    r: &PlannedRename,
    device_label: Option<&str>,
    now_ms: i64,
) -> Result<Option<Conflict>, SyncError> {
    let mut conflict = None;
    if vault_fs.exists(config, &r.new_vault_path).await? {
        if !vault_fs.exists(config, &r.old_vault_path).await? {
            return Ok(None);
        }
        let sidecar = crate::conflict::sidecar_path(
            vault_fs,
            config,
            &r.new_vault_path,
            device_label,
            now_ms,
        )
        .await?;
        match vault_fs.rename(config, &r.new_vault_path, &sidecar).await {
            Ok(()) | Err(VfsError::NotFound) => {}
            Err(e) => return Err(e.into()),
        }
        conflict = Some(Conflict {
            node_id: r.node_id,
            vault_path: r.new_vault_path.clone(),
            reason: cu_msg::Msg::new("CONFLICT_RENAME_TARGET_TAKEN").wire(),
            kind: ConflictKind::NewFileCollision,
            resolution: ConflictResolution::KeptBoth {
                sidecar_path: sidecar,
            },
        });
    }
    match vault_fs
        .rename(config, &r.old_vault_path, &r.new_vault_path)
        .await
    {
        Ok(()) => Ok(conflict),
        Err(VfsError::NotFound) => {
            if vault_fs.exists(config, &r.new_vault_path).await? {
                Ok(conflict)
            } else {
                Err(VfsError::NotFound.into())
            }
        }
        Err(e) => Err(e.into()),
    }
}

async fn vault_subtree_has_files(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_path: &str,
) -> Result<bool, SyncError> {
    let mut stack = vec![vault_path.to_string()];
    while let Some(dir) = stack.pop() {
        let entries = match vault_fs.list(config, &dir).await {
            Ok(entries) => entries,
            Err(VfsError::NotFound) => continue,
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            match entry.kind {
                cu_vfs::NodeKind::File => return Ok(true),
                cu_vfs::NodeKind::Dir => stack.push(format!("{dir}/{}", entry.name)),
            }
        }
    }
    Ok(false)
}

async fn execute_delete(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    d: &PlannedDelete,
    must_free_path: bool,
    device_label: Option<&str>,
    now_ms: i64,
) -> Result<Option<Conflict>, SyncError> {
    match d.kind {
        cu_index::NodeKind::Dir => {
            if vault_subtree_has_files(vault_fs, config, &d.vault_path).await? {
                let kept_at = if must_free_path {
                    let sidecar = crate::conflict::sidecar_path(
                        vault_fs,
                        config,
                        &d.vault_path,
                        device_label,
                        now_ms,
                    )
                    .await?;
                    match vault_fs.rename(config, &d.vault_path, &sidecar).await {
                        Ok(()) | Err(VfsError::NotFound) => {}
                        Err(e) => return Err(e.into()),
                    }
                    sidecar
                } else {
                    d.vault_path.clone()
                };
                return Ok(Some(Conflict {
                    node_id: d.node_id,
                    vault_path: d.vault_path.clone(),
                    reason: cu_msg::Msg::new("CONFLICT_FOLDER_HAS_NEW_FILES").wire(),
                    kind: ConflictKind::DeleteVsModify,
                    resolution: ConflictResolution::KeptBoth {
                        sidecar_path: kept_at,
                    },
                }));
            }
            match vault_fs.remove_dir(config, &d.vault_path).await {
                Ok(()) | Err(VfsError::NotFound) => Ok(None),
                Err(e) => Err(e.into()),
            }
        }
        cu_index::NodeKind::File => {
            if let Some(version) = d.archive_previous {
                let (versions_dir, archive_path) = archive_dir_and_path(d.node_id, version);
                vault_fs.mkdir(config, &versions_dir).await?;
                match vault_fs.rename(config, &d.vault_path, &archive_path).await {
                    Ok(()) | Err(VfsError::NotFound) => Ok(None),
                    Err(e) => Err(e.into()),
                }
            } else {
                match vault_fs.remove_file(config, &d.vault_path).await {
                    Ok(()) | Err(VfsError::NotFound) => Ok(None),
                    Err(e) => Err(e.into()),
                }
            }
        }
    }
}

fn commit_upload(
    db: &mut Db,
    root_id: RootId,
    u: &PlannedUpload,
    now_ms: i64,
    header_nonce: Option<HeaderNonce>,
) -> Result<(), SyncError> {
    if let Some((old_version, _)) = u.archive_previous {
        db.set_version_location(u.node_id, old_version, VersionLocation::Archived)?;
        let new_version = old_version + 1;
        insert_version_idempotent(
            db,
            &VersionRow {
                node_id: u.node_id,
                version: new_version,
                created_at: now_ms,
                kind: cu_index::NodeKind::File,
                parent_id: u.parent_id,
                name: u.name.clone(),
                size: u.size,
                content_hash: Some(u.content_hash),
                location: VersionLocation::Canonical,
                deleted: false,
            },
        )?;
        db.rename_node(u.node_id, u.parent_id, &u.name)?;
        db.update_baseline(
            u.node_id,
            u.size,
            u.mtime_ns,
            Some(u.content_hash),
            new_version,
            Some(u.scan),
            header_nonce,
        )?;
    } else {
        insert_node_idempotent(
            db,
            &NodeRow {
                node_id: u.node_id,
                root_id,
                parent_id: u.parent_id,
                name: u.name.clone(),
                kind: cu_index::NodeKind::File,
                size: u.size,
                mtime_ns: u.mtime_ns,
                content_hash: Some(u.content_hash),
                version: 0,
                scan: Some(u.scan),
                header_nonce,
            },
        )?;
        insert_version_idempotent(
            db,
            &VersionRow {
                node_id: u.node_id,
                version: 0,
                created_at: now_ms,
                kind: cu_index::NodeKind::File,
                parent_id: u.parent_id,
                name: u.name.clone(),
                size: u.size,
                content_hash: Some(u.content_hash),
                location: VersionLocation::Canonical,
                deleted: false,
            },
        )?;
    }
    Ok(())
}
