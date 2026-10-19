// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::path::Path;

use cu_format::config::VaultConfig;
use cu_index::{Db, NodeId, NodeRow, VersionLocation, VersionRow};
use cu_vfs::VaultFs;

use crate::error::SyncError;
use crate::execute::archive_vault_path;

pub async fn restore_version(
    db: &Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_path_prefix: &str,
    node_id: NodeId,
    version: i64,
    local_dest: &Path,
) -> Result<(), SyncError> {
    let source_path = resolve_restore_source(db, vault_path_prefix, node_id, version)?;
    write_restored_file(vault_fs, config, &source_path, local_dest).await
}

pub fn resolve_restore_source(
    db: &Db,
    vault_path_prefix: &str,
    node_id: NodeId,
    version: i64,
) -> Result<String, SyncError> {
    let versions = db.list_versions(node_id)?;
    let target = versions
        .iter()
        .find(|v| v.version == version && !v.deleted)
        .ok_or(SyncError::VersionNotFound)?;
    resolve_source_path(db, vault_path_prefix, &versions, target)
}

pub async fn write_restored_file(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    source_path: &str,
    local_dest: &Path,
) -> Result<(), SyncError> {
    use tokio::io::AsyncWriteExt as _;

    let parent = local_dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let mut suffix = [0u8; 8];
    cu_crypto::random::fill_random(&mut suffix)?;
    let file_name = local_dest
        .file_name()
        .map_or_else(|| "restore".into(), |n| n.to_string_lossy().into_owned());
    let temp = parent.join(format!(
        ".{file_name}.cu-restore-{}",
        crate::hexutil::encode(&suffix)
    ));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .await?;
        vault_fs
            .read_file(config, source_path, &mut file, &mut |_| {})
            .await?;
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temp, local_dest).await?;
        Ok::<(), SyncError>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result
}

pub async fn restore_before(
    db: &Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_path_prefix: &str,
    node_id: NodeId,
    at_ms: i64,
    local_dest: &Path,
) -> Result<(), SyncError> {
    let source_path = resolve_restore_source_before(db, vault_path_prefix, node_id, at_ms)?;
    write_restored_file(vault_fs, config, &source_path, local_dest).await
}

pub fn resolve_restore_source_before(
    db: &Db,
    vault_path_prefix: &str,
    node_id: NodeId,
    at_ms: i64,
) -> Result<String, SyncError> {
    let versions = db.list_versions(node_id)?;
    let target = versions
        .iter()
        .find(|v| v.created_at <= at_ms && !v.deleted)
        .ok_or(SyncError::VersionNotFound)?;
    resolve_source_path(db, vault_path_prefix, &versions, target)
}

const MAX_ANCESTOR_DEPTH: usize = 4096;

pub(crate) fn logical_path_of(
    db: &Db,
    vault_path_prefix: &str,
    node_id: NodeId,
) -> Result<Option<String>, SyncError> {
    let mut segments = Vec::new();
    let mut current = db.get_node(node_id)?;
    if current.is_none() {
        return Ok(None);
    }
    while let Some(node) = current {
        if segments.len() >= MAX_ANCESTOR_DEPTH {
            return Err(SyncError::IntegrityFailure(cu_msg::Msg::new(
                "SYNC_STATE_CYCLE",
            )));
        }
        segments.push(node.name.clone());
        current = match node.parent_id {
            Some(parent) => db.get_node(parent)?,
            None => None,
        };
    }
    segments.reverse();
    let relative = segments.join("/");
    Ok(Some(if vault_path_prefix.is_empty() {
        relative
    } else {
        format!("{vault_path_prefix}/{relative}")
    }))
}

pub(crate) fn logical_path_of_indexed(
    by_id: &HashMap<NodeId, &NodeRow>,
    vault_path_prefix: &str,
    node_id: NodeId,
) -> Option<String> {
    let mut segments = Vec::new();
    let mut current = *by_id.get(&node_id)?;
    loop {
        if segments.len() >= MAX_ANCESTOR_DEPTH {
            return None;
        }
        segments.push(current.name.clone());
        match current.parent_id {
            Some(parent_id) => current = by_id.get(&parent_id)?,
            None => break,
        }
    }
    segments.reverse();
    let relative = segments.join("/");
    Some(if vault_path_prefix.is_empty() {
        relative
    } else {
        format!("{vault_path_prefix}/{relative}")
    })
}

fn resolve_source_path(
    db: &Db,
    vault_path_prefix: &str,
    versions: &[VersionRow],
    target: &VersionRow,
) -> Result<String, SyncError> {
    match target.location {
        VersionLocation::Archived => Ok(archive_vault_path(target.node_id, target.version)),
        VersionLocation::Canonical => {
            let current = db.get_node(target.node_id)?;
            let current_version = current.as_ref().map(|n| n.version);
            if current_version == Some(target.version)
                && let Some(path) = logical_path_of(db, vault_path_prefix, target.node_id)?
            {
                return Ok(path);
            }
            let Some(same_content) = versions
                .iter()
                .filter(|v| {
                    v.version > target.version
                        && v.content_hash == target.content_hash
                        && target.content_hash.is_some()
                        && !v.deleted
                        && (v.location == VersionLocation::Archived
                            || Some(v.version) == current_version)
                })
                .min_by_key(|v| v.version)
            else {
                return Err(SyncError::ContentUnavailable);
            };
            if same_content.location == VersionLocation::Archived {
                Ok(archive_vault_path(
                    same_content.node_id,
                    same_content.version,
                ))
            } else {
                logical_path_of(db, vault_path_prefix, target.node_id)?
                    .ok_or(SyncError::ContentUnavailable)
            }
        }
    }
}
