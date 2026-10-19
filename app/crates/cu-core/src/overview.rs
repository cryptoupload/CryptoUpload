// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::path::Path;

use cu_jobs::{JobSpec, VaultRef};

use crate::CoreError;
use crate::jobs::{default_jobs_path, default_status_dir, job_state_db_path};
use crate::local_state::LocalStateAccess;

fn canonical_or(p: &Path) -> std::path::PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

#[must_use]
pub fn same_vault(a: &VaultRef, b: &VaultRef) -> bool {
    match (a, b) {
        (VaultRef::Local(pa), VaultRef::Local(pb)) => canonical_or(pa) == canonical_or(pb),
        (VaultRef::Remote(na), VaultRef::Remote(nb)) => na == nb,
        _ => false,
    }
}

#[must_use]
pub fn vault_ref_from_location(location: &str) -> VaultRef {
    location.strip_prefix("remote:").map_or_else(
        || VaultRef::Local(location.into()),
        |name| VaultRef::Remote(name.to_string()),
    )
}

fn jobs_of(data_dir: &Path, vault: &VaultRef) -> Result<Vec<JobSpec>, CoreError> {
    let specs = cu_jobs::JobStore::new(default_jobs_path(data_dir)).load()?;
    Ok(specs
        .into_iter()
        .filter(|s| same_vault(&s.vault, vault))
        .collect())
}

fn open_index(
    data_dir: &Path,
    spec: &JobSpec,
    access: &LocalStateAccess,
) -> Result<Option<(cu_index::Db, cu_index::RootId)>, CoreError> {
    let path = job_state_db_path(data_dir, spec)?;
    if !path.exists() {
        return Ok(None);
    }
    let db = cu_index::Db::open_reading(&path, access.cipher())?;
    let root = db.find_root(&spec.local_root.to_string_lossy(), &spec.vault_path)?;
    Ok(root.map(|r| (db, r)))
}

const MAX_DEPTH: usize = 4096;

fn node_path(db: &cu_index::Db, prefix: &str, node: &cu_index::NodeRow) -> Option<String> {
    let mut segments = vec![node.name.clone()];
    let mut parent = node.parent_id;
    while let Some(id) = parent {
        if segments.len() >= MAX_DEPTH {
            return None;
        }
        let p = db.get_node(id).ok()??;
        segments.push(p.name);
        parent = p.parent_id;
    }
    if !prefix.is_empty() {
        segments.push(prefix.to_string());
    }
    segments.reverse();
    Some(segments.join("/"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentFile {
    pub job: String,
    pub path: String,
    pub size: u64,
    pub modified: i64,
}

pub fn recent_files(
    data_dir: &Path,
    vault: &VaultRef,
    limit: u32,
    access: &LocalStateAccess,
) -> Result<Vec<RecentFile>, CoreError> {
    let mut out = Vec::new();
    for spec in jobs_of(data_dir, vault)? {
        let Some((db, root)) = open_index(data_dir, &spec, access)? else {
            continue;
        };
        if db.is_sealed() && access.cipher().is_none() {
            continue;
        }
        for node in db.recent_files(root, limit)? {
            if let Some(path) = node_path(&db, &spec.vault_path, &node) {
                out.push(RecentFile {
                    job: spec.name.clone(),
                    path,
                    size: node.size,
                    modified: node.mtime_ns.div_euclid(1_000_000_000),
                });
            }
        }
    }
    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    out.truncate(limit as usize);
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySyncState {
    Synced,
    Pending,
    Conflict,
}

fn split(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

pub fn sync_states(
    data_dir: &Path,
    vault: &VaultRef,
    dir: &str,
    names: &[String],
    access: &LocalStateAccess,
) -> Result<HashMap<String, EntrySyncState>, CoreError> {
    let mut out = HashMap::new();
    let dir_segs = split(dir);
    for spec in jobs_of(data_dir, vault)? {
        let prefix = split(&spec.vault_path);
        if prefix.len() > dir_segs.len() && prefix[..dir_segs.len()] == dir_segs[..] {
            out.insert(prefix[dir_segs.len()].to_string(), EntrySyncState::Synced);
            continue;
        }
        if dir_segs.len() < prefix.len() || dir_segs[..prefix.len()] != prefix[..] {
            continue;
        }
        let Some((db, root)) = open_index(data_dir, &spec, access)? else {
            continue;
        };
        if db.is_sealed() && access.cipher().is_none() {
            continue;
        }
        let mut parent = None;
        let mut found = true;
        for seg in &dir_segs[prefix.len()..] {
            if let Some(n) = db.find_child(root, parent, &cu_format::name::nfc(seg))? {
                parent = Some(n.node_id);
            } else {
                found = false;
                break;
            }
        }
        if !found {
            continue;
        }
        let pending: std::collections::HashSet<cu_index::NodeId> = db
            .journal_pending(root)?
            .into_iter()
            .map(|j| j.node_id)
            .collect();
        for child in db.list_children(root, parent)? {
            let state = if pending.contains(&child.node_id) {
                EntrySyncState::Pending
            } else {
                EntrySyncState::Synced
            };
            out.insert(child.name, state);
        }
    }
    for name in names {
        if cu_sync::conflict_original_name(name).is_some() {
            out.insert(name.clone(), EntrySyncState::Conflict);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceJobStatus {
    pub job: String,
    pub service_active: bool,
    pub next_run: Option<i64>,
    pub watcher_health: String,
}

pub fn service_status(
    data_dir: &Path,
    vault: &VaultRef,
    now_ms: i64,
) -> Result<Vec<ServiceJobStatus>, CoreError> {
    let store = cu_jobs::StatusStore::new(default_status_dir(data_dir));
    Ok(jobs_of(data_dir, vault)?
        .into_iter()
        .map(|spec| {
            let interval_ms = i64::try_from(
                cu_jobs::scheduler::effective_interval(spec.interval_secs).as_millis(),
            )
            .unwrap_or(i64::MAX);
            let status = store.read(&spec.name).ok().flatten();
            let active = spec.enabled
                && status
                    .as_ref()
                    .is_some_and(|s| now_ms - s.updated_at_ms <= interval_ms.saturating_mul(3));
            let next_run = status
                .as_ref()
                .filter(|_| active)
                .and_then(|s| s.last_run_started_at_ms)
                .map(|started| (started + interval_ms).max(now_ms).div_euclid(1000));
            ServiceJobStatus {
                job: spec.name,
                service_active: active,
                next_run,
                watcher_health: status.map_or_else(|| "healthy".to_string(), |s| s.watcher_health),
            }
        })
        .collect())
}
