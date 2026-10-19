// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{HashMap, HashSet};
use std::path::Path;

use cu_format::config::VaultConfig;
use cu_index::{NodeId, NodeKind, NodeRow, ScanFingerprint};
use cu_vfs::{HeaderNonce, VaultFs};
use futures_util::StreamExt as _;

use crate::conflict::sidecar_path;
use crate::cuignore::CuIgnore;
use crate::error::SyncError;
use crate::model::{
    Conflict, ConflictKind, ConflictResolution, Plan, PlannedDelete, PlannedMkdir, PlannedRename,
    PlannedSetAside, PlannedUpload,
};

const DRIFT_CHECK_CONCURRENCY: usize = 8;

const ANTI_RACE_WINDOW_NS: i64 = 2_000_000_000;

struct BaselineIndex {
    by_id: HashMap<NodeId, NodeRow>,
    by_parent_name: HashMap<Option<NodeId>, HashMap<String, NodeId>>,
    by_dev_ino: HashMap<(u64, u64), NodeId>,
    children: HashMap<NodeId, Vec<NodeId>>,
    claimed: HashSet<NodeId>,
    vault_dir_exists: HashMap<String, bool>,
    vault_prefix: String,
}

impl BaselineIndex {
    fn build(nodes: Vec<NodeRow>, vault_prefix: &str) -> Self {
        let mut by_id = HashMap::with_capacity(nodes.len());
        let mut by_parent_name: HashMap<Option<NodeId>, HashMap<String, NodeId>> = HashMap::new();
        let mut by_dev_ino = HashMap::new();
        let mut children: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for n in nodes {
            if let Some(parent) = n.parent_id {
                children.entry(parent).or_default().push(n.node_id);
            }
            by_parent_name
                .entry(n.parent_id)
                .or_default()
                .insert(n.name.clone(), n.node_id);
            if let Some(scan) = n.scan
                && (scan.dev != 0 || scan.ino != 0)
            {
                by_dev_ino.insert((scan.dev, scan.ino), n.node_id);
            }
            by_id.insert(n.node_id, n);
        }
        Self {
            by_id,
            by_parent_name,
            by_dev_ino,
            children,
            claimed: HashSet::new(),
            vault_dir_exists: HashMap::new(),
            vault_prefix: vault_prefix.to_string(),
        }
    }

    fn lookup_by_parent_name(&self, parent_id: Option<NodeId>, name: &str) -> Option<NodeId> {
        self.by_parent_name.get(&parent_id)?.get(name).copied()
    }

    fn node_at(&self, vault_path: &str) -> Option<NodeId> {
        let relative = if self.vault_prefix.is_empty() {
            vault_path
        } else {
            vault_path
                .strip_prefix(self.vault_prefix.as_str())?
                .strip_prefix('/')?
        };
        let mut current = None;
        for segment in relative.split('/') {
            current = Some(self.lookup_by_parent_name(current, segment)?);
        }
        current
    }

    fn logical_path(&self, node_id: NodeId) -> String {
        let mut segments = Vec::new();
        let mut current = self.by_id.get(&node_id);
        while let Some(row) = current {
            if segments.len() >= 4096 {
                break;
            }
            segments.push(row.name.clone());
            current = row.parent_id.and_then(|p| self.by_id.get(&p));
        }
        segments.reverse();
        join_vault(&self.vault_prefix, &segments.join("/"))
    }
}

fn new_node_id() -> Result<NodeId, SyncError> {
    let mut buf = [0u8; 16];
    cu_crypto::random::fill_random(&mut buf)?;
    Ok(NodeId(buf))
}

#[cfg(unix)]
fn dev_ino(meta: &std::fs::Metadata, _path: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(windows)]
fn dev_ino(_meta: &std::fs::Metadata, path: &Path) -> (u64, u64) {
    winapi_util::Handle::from_path_any(path)
        .and_then(|handle| winapi_util::file::information(&handle))
        .map_or((0, 0), |info| {
            (info.volume_serial_number(), info.file_index())
        })
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(i64::MIN, |d| {
            i64::try_from(d.as_nanos()).unwrap_or(i64::MAX)
        })
}

const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn windows_illegal_name(name: &str, vault_path: &str) -> Option<String> {
    const RESERVED_CHARS: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

    let msg = if name.contains(RESERVED_CHARS) {
        cu_msg::Msg::new("SYNC_SKIP_WINDOWS_CHARACTER")
    } else if name.ends_with('.') || name.ends_with(' ') {
        cu_msg::Msg::new("SYNC_SKIP_WINDOWS_TRAILING")
    } else if WINDOWS_RESERVED_NAMES.iter().any(|reserved| {
        name.split('.')
            .next()
            .unwrap_or(name)
            .eq_ignore_ascii_case(reserved)
    }) {
        cu_msg::Msg::new("SYNC_SKIP_WINDOWS_RESERVED")
    } else {
        return None;
    };
    Some(msg.arg(vault_path).wire())
}

fn unstorable_message(reason: cu_format::name::Unstorable, vault_path: &str) -> String {
    use cu_format::name::Unstorable;
    match reason {
        Unstorable::TooLong => cu_msg::Msg::new("SYNC_SKIP_NAME_TOO_LONG"),
        Unstorable::TooLongPrivate => cu_msg::Msg::new("SYNC_SKIP_NAME_TOO_LONG_PRIVATE"),
        Unstorable::ReservedByte => cu_msg::Msg::new("SYNC_SKIP_NAME_RESERVED_BYTE"),
    }
    .arg(vault_path)
    .wire()
}

fn join_vault(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn content_likely_unchanged(
    baseline_scan: Option<ScanFingerprint>,
    observed: ScanFingerprint,
    scan_started_at_ns: i64,
) -> bool {
    let Some(baseline) = baseline_scan else {
        return false;
    };
    if observed.mtime_ns == i64::MIN || baseline.mtime_ns == i64::MIN {
        return false;
    }
    if observed.mtime_ns > scan_started_at_ns - ANTI_RACE_WINDOW_NS {
        return false;
    }
    baseline.dev == observed.dev
        && baseline.ino == observed.ino
        && baseline.size == observed.size
        && baseline.mtime_ns == observed.mtime_ns
}

async fn hash_local_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = cu_crypto::hash::Hasher::new();
        let wanted = file
            .metadata()
            .map_or(256 * 1024, |m| m.len().saturating_add(1));
        let mut buf = vec![
            0u8;
            usize::try_from(wanted)
                .unwrap_or(usize::MAX)
                .min(256 * 1024)
        ];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize())
    })
    .await
    .expect("hashing a file must never panic")
}

pub(crate) enum VaultDrift {
    Changed(String),
    Vanished(String),
}

async fn check_vault_drift(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_path: &str,
    expected_hash: Option<[u8; 32]>,
    expected_nonce: Option<HeaderNonce>,
    local_hash: Option<[u8; 32]>,
) -> Result<Option<VaultDrift>, SyncError> {
    let Some(expected) = expected_hash else {
        return Ok(None);
    };
    if let Some(nonce) = expected_nonce
        && matches!(vault_fs.read_header_nonce(config, vault_path).await, Ok(found) if found == nonce)
    {
        return Ok(None);
    }
    let mut sink = crate::verify::HashingSink(cu_crypto::hash::Hasher::new());
    match vault_fs
        .read_file(config, vault_path, &mut sink, &mut |_| {})
        .await
    {
        Ok(()) => {
            let actual = sink.0.finalize();
            if actual == expected || Some(actual) == local_hash {
                Ok(None)
            } else {
                Ok(Some(VaultDrift::Changed(
                    cu_msg::Msg::new("CONFLICT_VAULT_CHANGED").wire(),
                )))
            }
        }
        Err(cu_vfs::VfsError::NotFound) => Ok(Some(VaultDrift::Vanished(
            cu_msg::Msg::new("CONFLICT_VAULT_VANISHED").wire(),
        ))),
        Err(e) => Err(e.into()),
    }
}

async fn vault_dir_is_missing(
    idx: &mut BaselineIndex,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_dir: &str,
) -> Result<bool, SyncError> {
    if vault_dir.is_empty() {
        return Ok(false);
    }
    let mut ancestor = vault_dir;
    loop {
        if idx.vault_dir_exists.get(ancestor) == Some(&false) {
            return Ok(true);
        }
        match ancestor.rfind('/') {
            Some(cut) => ancestor = &ancestor[..cut],
            None => break,
        }
    }
    if let Some(&exists) = idx.vault_dir_exists.get(vault_dir) {
        return Ok(!exists);
    }
    let exists = match vault_fs.resolve_dir_ref(config, vault_dir).await {
        Ok(_) => true,
        Err(cu_vfs::VfsError::NotFound) => false,
        Err(e) => return Err(e.into()),
    };
    idx.vault_dir_exists.insert(vault_dir.to_string(), exists);
    Ok(!exists)
}

async fn vault_already_holds(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_path: &str,
    hash: [u8; 32],
) -> Result<bool, SyncError> {
    let mut sink = crate::verify::HashingSink(cu_crypto::hash::Hasher::new());
    match vault_fs
        .read_file(config, vault_path, &mut sink, &mut |_| {})
        .await
    {
        Ok(()) => Ok(sink.0.finalize() == hash),
        Err(cu_vfs::VfsError::NotFound) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

enum LocalSkip {
    Vanished,
    Denied,
}

async fn read_dir_with_metadata(
    dir: &Path,
) -> std::io::Result<
    Vec<(
        std::ffi::OsString,
        std::path::PathBuf,
        std::io::Result<std::fs::Metadata>,
    )>,
> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::fs::read_dir(&dir)?
            .map(|entry| {
                let entry = entry?;
                Ok((entry.file_name(), entry.path(), entry.metadata()))
            })
            .collect()
    })
    .await
    .map_err(std::io::Error::other)?
}

fn local_skip(err: &std::io::Error) -> Option<LocalSkip> {
    match err.kind() {
        std::io::ErrorKind::NotFound => Some(LocalSkip::Vanished),
        std::io::ErrorKind::PermissionDenied => Some(LocalSkip::Denied),
        _ => None,
    }
}

fn denied_message(vault_path: &str) -> String {
    cu_msg::Msg::new("SYNC_SKIP_READ_DENIED")
        .arg(vault_path)
        .wire()
}

fn skip_or_fail(err: std::io::Error, vault_path: &str, plan: &mut Plan) -> Result<(), SyncError> {
    match local_skip(&err) {
        Some(LocalSkip::Vanished) => Ok(()),
        Some(LocalSkip::Denied) => {
            plan.skipped.push(denied_message(vault_path));
            Ok(())
        }
        None => Err(err.into()),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn build_plan(
    local_root: &Path,
    vault_prefix: &str,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    all_nodes: Vec<NodeRow>,
    scan_started_at_ns: i64,
    ignore: &CuIgnore,
    forget_ignored: bool,
    device_label: Option<&str>,
    now_ms: i64,
) -> Result<Plan, SyncError> {
    let mut idx = BaselineIndex::build(all_nodes, vault_prefix);
    let mut plan = Plan {
        vault_prefix: vault_prefix.to_string(),
        ..Plan::default()
    };
    resolve_dir(
        local_root,
        vault_prefix,
        "",
        None,
        &mut idx,
        vault_fs,
        config,
        scan_started_at_ns,
        ignore,
        forget_ignored,
        device_label,
        now_ms,
        &mut plan,
    )
    .await?;
    collect_deletes(&mut idx, vault_fs, config, device_label, now_ms, &mut plan).await?;
    Ok(plan)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn resolve_dir<'a>(
    local_dir: &'a Path,
    vault_dir: &'a str,
    local_rel: &'a str,
    parent_id: Option<NodeId>,
    idx: &'a mut BaselineIndex,
    vault_fs: &'a VaultFs,
    config: &'a VaultConfig,
    scan_started_at_ns: i64,
    ignore: &'a CuIgnore,
    forget_ignored: bool,
    device_label: Option<&'a str>,
    now_ms: i64,
    plan: &'a mut Plan,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), SyncError>> + Send + 'a>> {
    Box::pin(async move {
        let mut seen_lowercase: HashSet<String> = HashSet::new();
        let entries = match read_dir_with_metadata(local_dir).await {
            Ok(entries) => entries,
            Err(e) if parent_id.is_some() && local_skip(&e).is_some() => {
                if matches!(local_skip(&e), Some(LocalSkip::Denied)) {
                    plan.skipped.push(denied_message(vault_dir));
                }
                if let Some(dir_id) = parent_id {
                    claim_subtree(idx, dir_id);
                }
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        for (os_name, local_path, meta) in entries {
            let meta = match meta {
                Ok(meta) => meta,
                Err(e) => match local_skip(&e) {
                    Some(LocalSkip::Vanished) => continue,
                    Some(LocalSkip::Denied) => {
                        let name = os_name.to_string_lossy();
                        plan.skipped
                            .push(denied_message(&join_vault(vault_dir, &name)));
                        if let Some(name) = os_name.to_str() {
                            claim_local_survivor(idx, parent_id, Some(name), 0, 0);
                        }
                        continue;
                    }
                    None => return Err(e.into()),
                },
            };
            let (dev, ino) = dev_ino(&meta, &local_path);
            let Some(name) = os_name.to_str() else {
                plan.skipped.push(
                    cu_msg::Msg::new("SYNC_SKIP_NAME_NOT_UTF8")
                        .arg(join_vault(vault_dir, &os_name.to_string_lossy()))
                        .wire(),
                );
                claim_local_survivor(idx, parent_id, None, dev, ino);
                continue;
            };
            let raw_name = name;
            let name = cu_format::name::nfc(raw_name).into_owned();
            if vault_dir.is_empty() && name == ".cryptoupload" {
                return Err(SyncError::ReservedName(name));
            }
            if let Some(reason) = windows_illegal_name(&name, &join_vault(vault_dir, &name)) {
                plan.skipped.push(reason);
                claim_local_survivor(idx, parent_id, Some(&name), dev, ino);
                continue;
            }
            if let Some(reason) = cu_format::name::unstorable_reason(config.profile(), &name) {
                plan.skipped
                    .push(unstorable_message(reason, &join_vault(vault_dir, &name)));
                claim_local_survivor(idx, parent_id, Some(&name), dev, ino);
                continue;
            }
            if !seen_lowercase.insert(name.to_lowercase()) {
                plan.skipped.push(
                    cu_msg::Msg::new("SYNC_SKIP_CASE_COLLISION")
                        .arg(join_vault(vault_dir, &name))
                        .wire(),
                );
                claim_local_survivor(idx, parent_id, Some(&name), dev, ino);
                continue;
            }
            if meta.file_type().is_symlink() {
                claim_local_survivor(idx, parent_id, Some(&name), dev, ino);
                continue;
            }
            let size = meta.len();
            let mtime = mtime_ns(&meta);
            let is_dir = meta.is_dir();
            let local_kind = if is_dir {
                NodeKind::Dir
            } else {
                NodeKind::File
            };
            let vault_path = join_vault(vault_dir, &name);
            let rel_path = join_vault(local_rel, raw_name);
            let observed_scan = ScanFingerprint {
                dev,
                ino,
                size,
                mtime_ns: mtime,
            };

            if ignore.matched(&rel_path, is_dir) {
                if !forget_ignored
                    && let Some(baseline_id) = idx.lookup_by_parent_name(parent_id, &name)
                {
                    reclaim_ignored_subtree(idx, baseline_id, plan);
                }
                continue;
            }

            let same_position = idx
                .lookup_by_parent_name(parent_id, &name)
                .and_then(|id| idx.by_id.get(&id).cloned())
                .filter(|b| b.kind == local_kind && !idx.claimed.contains(&b.node_id));

            if let Some(baseline) = same_position {
                idx.claimed.insert(baseline.node_id);
                if is_dir {
                    resolve_dir(
                        &local_path,
                        &vault_path,
                        &rel_path,
                        Some(baseline.node_id),
                        idx,
                        vault_fs,
                        config,
                        scan_started_at_ns,
                        ignore,
                        forget_ignored,
                        device_label,
                        now_ms,
                        plan,
                    )
                    .await?;
                } else if !content_likely_unchanged(
                    baseline.scan,
                    observed_scan,
                    scan_started_at_ns,
                ) {
                    let content_hash = match hash_local_file(&local_path).await {
                        Ok(hash) => hash,
                        Err(e) => {
                            skip_or_fail(e, &vault_path, plan)?;
                            continue;
                        }
                    };
                    if Some(content_hash) != baseline.content_hash {
                        let baseline_path = idx.logical_path(baseline.node_id);
                        match check_vault_drift(
                            vault_fs,
                            config,
                            &baseline_path,
                            baseline.content_hash,
                            baseline.header_nonce,
                            Some(content_hash),
                        )
                        .await?
                        {
                            Some(VaultDrift::Changed(reason)) => {
                                let sidecar = sidecar_path(
                                    vault_fs,
                                    config,
                                    &baseline_path,
                                    device_label,
                                    now_ms,
                                )
                                .await?;
                                plan.set_asides.push(PlannedSetAside {
                                    node_id: baseline.node_id,
                                    vault_path: baseline_path.clone(),
                                    sidecar_path: sidecar.clone(),
                                    forget_node: false,
                                });
                                plan.conflicts.push(Conflict {
                                    node_id: baseline.node_id,
                                    vault_path: baseline_path.clone(),
                                    reason,
                                    kind: ConflictKind::ModifiedBothSides,
                                    resolution: ConflictResolution::KeptBoth {
                                        sidecar_path: sidecar,
                                    },
                                });
                                plan.uploads.push(PlannedUpload {
                                    node_id: baseline.node_id,
                                    parent_id,
                                    name,
                                    vault_path: vault_path.clone(),
                                    local_path,
                                    size,
                                    mtime_ns: mtime,
                                    scan: observed_scan,
                                    content_hash,
                                    archive_previous: Some((baseline.version, vault_path)),
                                    skip_archive: true,
                                });
                            }
                            Some(VaultDrift::Vanished(reason)) => {
                                plan.conflicts.push(Conflict {
                                    node_id: baseline.node_id,
                                    vault_path: vault_path.clone(),
                                    reason,
                                    kind: ConflictKind::VaultVanished,
                                    resolution: ConflictResolution::Resurrected,
                                });
                                plan.uploads.push(PlannedUpload {
                                    node_id: baseline.node_id,
                                    parent_id,
                                    name,
                                    vault_path: vault_path.clone(),
                                    local_path,
                                    size,
                                    mtime_ns: mtime,
                                    scan: observed_scan,
                                    content_hash,
                                    archive_previous: Some((baseline.version, vault_path)),
                                    skip_archive: true,
                                });
                            }
                            None => {
                                plan.uploads.push(PlannedUpload {
                                    node_id: baseline.node_id,
                                    parent_id,
                                    name,
                                    vault_path: vault_path.clone(),
                                    local_path,
                                    size,
                                    mtime_ns: mtime,
                                    scan: observed_scan,
                                    content_hash,
                                    archive_previous: Some((baseline.version, vault_path)),
                                    skip_archive: false,
                                });
                            }
                        }
                    }
                }
                continue;
            }

            let rename_candidate = if dev == 0 && ino == 0 {
                None
            } else {
                idx.by_dev_ino
                    .get(&(dev, ino))
                    .copied()
                    .and_then(|id| idx.by_id.get(&id).cloned())
                    .filter(|c| !idx.claimed.contains(&c.node_id) && c.kind == local_kind)
            };

            if let Some(candidate) = rename_candidate {
                idx.claimed.insert(candidate.node_id);
                let old_vault_path = idx.logical_path(candidate.node_id);
                if is_dir {
                    plan.renames.push(PlannedRename {
                        node_id: candidate.node_id,
                        old_parent_id: candidate.parent_id,
                        old_name: candidate.name.clone(),
                        old_vault_path,
                        new_parent_id: parent_id,
                        new_name: name,
                        new_vault_path: vault_path.clone(),
                        kind: NodeKind::Dir,
                    });
                    resolve_dir(
                        &local_path,
                        &vault_path,
                        &rel_path,
                        Some(candidate.node_id),
                        idx,
                        vault_fs,
                        config,
                        scan_started_at_ns,
                        ignore,
                        forget_ignored,
                        device_label,
                        now_ms,
                        plan,
                    )
                    .await?;
                } else if content_likely_unchanged(
                    candidate.scan,
                    observed_scan,
                    scan_started_at_ns,
                ) {
                    plan.renames.push(PlannedRename {
                        node_id: candidate.node_id,
                        old_parent_id: candidate.parent_id,
                        old_name: candidate.name.clone(),
                        old_vault_path,
                        new_parent_id: parent_id,
                        new_name: name,
                        new_vault_path: vault_path,
                        kind: NodeKind::File,
                    });
                } else {
                    let content_hash = match hash_local_file(&local_path).await {
                        Ok(hash) => hash,
                        Err(e) => {
                            skip_or_fail(e, &vault_path, plan)?;
                            continue;
                        }
                    };
                    if Some(content_hash) == candidate.content_hash {
                        plan.renames.push(PlannedRename {
                            node_id: candidate.node_id,
                            old_parent_id: candidate.parent_id,
                            old_name: candidate.name.clone(),
                            old_vault_path,
                            new_parent_id: parent_id,
                            new_name: name,
                            new_vault_path: vault_path,
                            kind: NodeKind::File,
                        });
                    } else {
                        let mut drift = check_vault_drift(
                            vault_fs,
                            config,
                            &old_vault_path,
                            candidate.content_hash,
                            candidate.header_nonce,
                            Some(content_hash),
                        )
                        .await?;
                        if drift.is_some()
                            && vault_already_holds(vault_fs, config, &vault_path, content_hash)
                                .await?
                        {
                            drift = None;
                        }
                        match drift {
                            Some(VaultDrift::Changed(reason)) => {
                                let sidecar = sidecar_path(
                                    vault_fs,
                                    config,
                                    &old_vault_path,
                                    device_label,
                                    now_ms,
                                )
                                .await?;
                                plan.set_asides.push(PlannedSetAside {
                                    node_id: candidate.node_id,
                                    vault_path: old_vault_path.clone(),
                                    sidecar_path: sidecar.clone(),
                                    forget_node: false,
                                });
                                plan.conflicts.push(Conflict {
                                    node_id: candidate.node_id,
                                    vault_path: old_vault_path.clone(),
                                    reason,
                                    kind: ConflictKind::ModifiedBothSides,
                                    resolution: ConflictResolution::KeptBoth {
                                        sidecar_path: sidecar,
                                    },
                                });
                                plan.uploads.push(PlannedUpload {
                                    node_id: candidate.node_id,
                                    parent_id,
                                    name,
                                    vault_path: vault_path.clone(),
                                    local_path,
                                    size,
                                    mtime_ns: mtime,
                                    scan: observed_scan,
                                    content_hash,
                                    archive_previous: Some((candidate.version, old_vault_path)),
                                    skip_archive: true,
                                });
                            }
                            Some(VaultDrift::Vanished(reason)) => {
                                plan.conflicts.push(Conflict {
                                    node_id: candidate.node_id,
                                    vault_path: old_vault_path.clone(),
                                    reason,
                                    kind: ConflictKind::VaultVanished,
                                    resolution: ConflictResolution::Resurrected,
                                });
                                plan.uploads.push(PlannedUpload {
                                    node_id: candidate.node_id,
                                    parent_id,
                                    name,
                                    vault_path: vault_path.clone(),
                                    local_path,
                                    size,
                                    mtime_ns: mtime,
                                    scan: observed_scan,
                                    content_hash,
                                    archive_previous: Some((candidate.version, old_vault_path)),
                                    skip_archive: true,
                                });
                            }
                            None => {
                                plan.uploads.push(PlannedUpload {
                                    node_id: candidate.node_id,
                                    parent_id,
                                    name,
                                    vault_path: vault_path.clone(),
                                    local_path,
                                    size,
                                    mtime_ns: mtime,
                                    scan: observed_scan,
                                    content_hash,
                                    archive_previous: Some((candidate.version, old_vault_path)),
                                    skip_archive: false,
                                });
                            }
                        }
                    }
                }
                continue;
            }

            let node_id = new_node_id()?;
            if is_dir {
                plan.mkdirs.push(PlannedMkdir {
                    node_id,
                    parent_id,
                    name,
                    vault_path: vault_path.clone(),
                    scan: observed_scan,
                });
                resolve_dir(
                    &local_path,
                    &vault_path,
                    &rel_path,
                    Some(node_id),
                    idx,
                    vault_fs,
                    config,
                    scan_started_at_ns,
                    ignore,
                    forget_ignored,
                    device_label,
                    now_ms,
                    plan,
                )
                .await?;
            } else {
                let content_hash = match hash_local_file(&local_path).await {
                    Ok(hash) => hash,
                    Err(e) => {
                        skip_or_fail(e, &vault_path, plan)?;
                        continue;
                    }
                };
                let drift = if idx.node_at(&vault_path).is_some()
                    || vault_dir_is_missing(idx, vault_fs, config, vault_dir).await?
                {
                    None
                } else {
                    check_vault_drift(
                        vault_fs,
                        config,
                        &vault_path,
                        Some(content_hash),
                        None,
                        None,
                    )
                    .await?
                };
                match drift {
                    Some(VaultDrift::Changed(reason)) => {
                        let sidecar =
                            sidecar_path(vault_fs, config, &vault_path, device_label, now_ms)
                                .await?;
                        plan.set_asides.push(PlannedSetAside {
                            node_id,
                            vault_path: vault_path.clone(),
                            sidecar_path: sidecar.clone(),
                            forget_node: false,
                        });
                        plan.conflicts.push(Conflict {
                            node_id,
                            vault_path: vault_path.clone(),
                            reason,
                            kind: ConflictKind::NewFileCollision,
                            resolution: ConflictResolution::KeptBoth {
                                sidecar_path: sidecar,
                            },
                        });
                        plan.uploads.push(PlannedUpload {
                            node_id,
                            parent_id,
                            name,
                            vault_path,
                            local_path,
                            size,
                            mtime_ns: mtime,
                            scan: observed_scan,
                            content_hash,
                            skip_archive: true,
                            archive_previous: None,
                        });
                    }
                    Some(VaultDrift::Vanished(_)) | None => {
                        plan.uploads.push(PlannedUpload {
                            node_id,
                            parent_id,
                            name,
                            vault_path,
                            local_path,
                            size,
                            mtime_ns: mtime,
                            scan: observed_scan,
                            content_hash,
                            skip_archive: false,
                            archive_previous: None,
                        });
                    }
                }
            }
        }
        Ok(())
    })
}

async fn collect_deletes(
    idx: &mut BaselineIndex,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    device_label: Option<&str>,
    now_ms: i64,
    plan: &mut Plan,
) -> Result<(), SyncError> {
    let unclaimed: Vec<NodeId> = idx
        .by_id
        .keys()
        .copied()
        .filter(|id| !idx.claimed.contains(id))
        .collect();
    let unclaimed_set: HashSet<NodeId> = unclaimed.iter().copied().collect();

    let mut ancestor_unclaimed_memo: HashMap<NodeId, bool> = HashMap::new();
    let mut is_descendant_of_unclaimed = |start: NodeId| -> bool {
        let mut chain = Vec::new();
        let mut id = start;
        let result = loop {
            if let Some(&cached) = ancestor_unclaimed_memo.get(&id) {
                break cached;
            }
            let Some(row) = idx.by_id.get(&id) else {
                break false;
            };
            let Some(parent) = row.parent_id else {
                break false;
            };
            if unclaimed_set.contains(&parent) {
                break true;
            }
            chain.push(id);
            id = parent;
        };
        for visited in chain {
            ancestor_unclaimed_memo.insert(visited, result);
        }
        result
    };

    for (file, drift) in check_deleted_files(idx, vault_fs, config, &unclaimed).await {
        let DeletedFile {
            id,
            version,
            vault_path,
            ..
        } = file;
        match drift? {
            Some(VaultDrift::Changed(reason)) => {
                let sidecar =
                    sidecar_path(vault_fs, config, &vault_path, device_label, now_ms).await?;
                plan.set_asides.push(PlannedSetAside {
                    node_id: id,
                    vault_path: vault_path.clone(),
                    sidecar_path: sidecar.clone(),
                    forget_node: true,
                });
                plan.conflicts.push(Conflict {
                    node_id: id,
                    vault_path,
                    reason,
                    kind: ConflictKind::DeleteVsModify,
                    resolution: ConflictResolution::KeptBoth {
                        sidecar_path: sidecar,
                    },
                });
            }
            Some(VaultDrift::Vanished(_)) | None => {
                plan.deletes.push(PlannedDelete {
                    node_id: id,
                    vault_path,
                    kind: NodeKind::File,
                    archive_previous: Some(version),
                });
            }
        }
    }

    let mut dirs: Vec<(String, NodeId)> = Vec::new();
    for &id in &unclaimed {
        let row = &idx.by_id[&id];
        if row.kind != NodeKind::Dir || is_descendant_of_unclaimed(id) {
            continue;
        }
        dirs.push((idx.logical_path(id), id));
    }
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    for (vault_path, id) in dirs {
        plan.deletes.push(PlannedDelete {
            node_id: id,
            vault_path,
            kind: NodeKind::Dir,
            archive_previous: None,
        });
    }
    Ok(())
}

struct DeletedFile {
    id: NodeId,
    version: i64,
    content_hash: Option<[u8; 32]>,
    header_nonce: Option<HeaderNonce>,
    vault_path: String,
}

async fn check_deleted_files(
    idx: &BaselineIndex,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    unclaimed: &[NodeId],
) -> Vec<(DeletedFile, Result<Option<VaultDrift>, SyncError>)> {
    let mut files: Vec<DeletedFile> = unclaimed
        .iter()
        .filter_map(|&id| {
            let row = &idx.by_id[&id];
            (row.kind == NodeKind::File).then(|| DeletedFile {
                id,
                version: row.version,
                content_hash: row.content_hash,
                header_nonce: row.header_nonce,
                vault_path: idx.logical_path(id),
            })
        })
        .collect();
    files.sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
    let checks: Vec<_> = files
        .iter()
        .map(|f| {
            check_vault_drift(
                vault_fs,
                config,
                &f.vault_path,
                f.content_hash,
                f.header_nonce,
                None,
            )
        })
        .collect();
    let drifts: Vec<_> = futures_util::stream::iter(checks)
        .buffered(DRIFT_CHECK_CONCURRENCY)
        .collect()
        .await;
    files.into_iter().zip(drifts).collect()
}

fn claim_subtree(idx: &mut BaselineIndex, node_id: NodeId) {
    let mut visited = HashSet::new();
    let mut stack = vec![node_id];
    while let Some(id) = stack.pop() {
        if !idx.by_id.contains_key(&id) || !visited.insert(id) {
            continue;
        }
        idx.claimed.insert(id);
        if let Some(kids) = idx.children.get(&id) {
            stack.extend(kids.iter().copied());
        }
    }
}

fn claim_local_survivor(
    idx: &mut BaselineIndex,
    parent_id: Option<NodeId>,
    name: Option<&str>,
    dev: u64,
    ino: u64,
) {
    let by_name = name.and_then(|n| idx.lookup_by_parent_name(parent_id, n));
    let by_dev_ino = if dev == 0 && ino == 0 {
        None
    } else {
        idx.by_dev_ino.get(&(dev, ino)).copied()
    };
    if let Some(node_id) = by_name.or(by_dev_ino) {
        claim_subtree(idx, node_id);
    }
}

fn reclaim_ignored_subtree(idx: &mut BaselineIndex, node_id: NodeId, plan: &mut Plan) {
    let mut visited = HashSet::new();
    let mut stack = vec![node_id];
    while let Some(id) = stack.pop() {
        let Some(kind) = idx.by_id.get(&id).map(|r| r.kind) else {
            continue;
        };
        if !visited.insert(id) {
            continue;
        }
        idx.claimed.insert(id);
        match kind {
            NodeKind::File => plan.ignored_kept.push(idx.logical_path(id)),
            NodeKind::Dir => {
                if let Some(kids) = idx.children.get(&id) {
                    stack.extend(kids.iter().copied());
                }
            }
        }
    }
}
