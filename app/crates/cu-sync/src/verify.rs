// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use cu_format::config::VaultConfig;
use cu_index::{Db, NodeId, NodeKind, NodeRow, RootId};
use cu_vfs::{DirRef, VaultFs, VfsError};
use futures_util::StreamExt;
use futures_util::stream;
use tokio::io::AsyncWrite;

use cu_msg::Msg;

use crate::error::SyncError;
use crate::restore::logical_path_of_indexed;

fn split_parent(path: &str) -> (&str, &str) {
    path.rsplit_once('/').map_or(("", path), |(p, n)| (p, n))
}

async fn resolve_files_by_parent_dir<'a>(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    by_id: &HashMap<NodeId, &'a NodeRow>,
    vault_path_prefix: &str,
    files: impl Iterator<Item = &'a NodeRow>,
) -> Result<(Vec<(&'a NodeRow, DirRef, String, String)>, Vec<VerifyIssue>), SyncError> {
    let mut by_parent: HashMap<String, Vec<(&NodeRow, String)>> = HashMap::new();
    for n in files {
        let Some(vault_path) = logical_path_of_indexed(by_id, vault_path_prefix, n.node_id) else {
            continue;
        };
        let parent_path = split_parent(&vault_path).0.to_string();
        by_parent
            .entry(parent_path)
            .or_default()
            .push((n, vault_path));
    }

    let mut prepared = Vec::new();
    let mut issues = Vec::new();
    for (parent_path, entries) in by_parent {
        match vault_fs.resolve_dir_ref(config, &parent_path).await {
            Ok(dir) => {
                for (n, vault_path) in entries {
                    let file_name = split_parent(&vault_path).1.to_string();
                    prepared.push((n, dir.clone(), file_name, vault_path));
                }
            }
            Err(VfsError::NotFound) => {
                for (n, vault_path) in entries {
                    issues.push(VerifyIssue {
                        node_id: n.node_id,
                        vault_path,
                        problem: Msg::new("VERIFY_MISSING").wire(),
                    });
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok((prepared, issues))
}

const VERIFY_CONCURRENCY: usize = 16;

pub(crate) struct HashingSink(pub(crate) cu_crypto::hash::Hasher);

impl AsyncWrite for HashingSink {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.0.update(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyIssue {
    pub node_id: NodeId,
    pub vault_path: String,
    pub problem: String,
}

pub async fn verify_existence(
    db: &Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_path_prefix: &str,
    root_id: RootId,
) -> Result<Vec<VerifyIssue>, SyncError> {
    let nodes = db.list_all_nodes(root_id)?;
    let by_id: HashMap<NodeId, &NodeRow> = nodes.iter().map(|n| (n.node_id, n)).collect();
    let files = nodes.iter().filter(|n| n.kind == NodeKind::File);
    let (prepared, mut issues) =
        resolve_files_by_parent_dir(vault_fs, config, &by_id, vault_path_prefix, files).await?;

    let mut in_flight = stream::iter(prepared)
        .map(|(n, dir, file_name, vault_path)| async move {
            let result = vault_fs.quick_size_in(config, &dir, &file_name).await;
            (n, vault_path, result)
        })
        .buffer_unordered(VERIFY_CONCURRENCY);

    while let Some((n, vault_path, result)) = in_flight.next().await {
        match result {
            Ok(cu_vfs::QuickSize::Bytes(size))
                if cu_vfs::expected_quick_size(config, n.size) != Some(size) =>
            {
                issues.push(VerifyIssue {
                    node_id: n.node_id,
                    vault_path,
                    problem: Msg::new("VERIFY_SIZE").arg(n.size).arg(size).wire(),
                });
            }
            Ok(cu_vfs::QuickSize::Bytes(_)) => {}
            Ok(cu_vfs::QuickSize::MalformedGeometry) => {
                issues.push(VerifyIssue {
                    node_id: n.node_id,
                    vault_path,
                    problem: Msg::new("VERIFY_GEOMETRY").wire(),
                });
            }
            Err(VfsError::NotFound) => issues.push(VerifyIssue {
                node_id: n.node_id,
                vault_path,
                problem: Msg::new("VERIFY_MISSING").wire(),
            }),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(issues)
}

pub async fn verify_content(
    db: &Db,
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_path_prefix: &str,
    root_id: RootId,
) -> Result<Vec<VerifyIssue>, SyncError> {
    let nodes = db.list_all_nodes(root_id)?;
    let by_id: HashMap<NodeId, &NodeRow> = nodes.iter().map(|n| (n.node_id, n)).collect();
    let files = nodes.iter().filter(|n| n.kind == NodeKind::File);
    let (prepared, mut issues) =
        resolve_files_by_parent_dir(vault_fs, config, &by_id, vault_path_prefix, files).await?;

    let mut in_flight = stream::iter(prepared)
        .map(|(n, dir, file_name, vault_path)| async move {
            let mut sink = HashingSink(cu_crypto::hash::Hasher::new());
            let result = vault_fs
                .read_file_in(config, &dir, &file_name, &mut sink, &mut |_| {})
                .await
                .map(|()| sink.0.finalize());
            (n, vault_path, result)
        })
        .buffer_unordered(VERIFY_CONCURRENCY);

    while let Some((n, vault_path, result)) = in_flight.next().await {
        match result {
            Ok(actual) => {
                if Some(actual) != n.content_hash {
                    issues.push(VerifyIssue {
                        node_id: n.node_id,
                        vault_path,
                        problem: Msg::new("VERIFY_HASH").wire(),
                    });
                }
            }
            Err(VfsError::NotFound) => issues.push(VerifyIssue {
                node_id: n.node_id,
                vault_path,
                problem: Msg::new("VERIFY_MISSING").wire(),
            }),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(issues)
}
