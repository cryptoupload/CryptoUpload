// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{BTreeSet, HashSet};
use std::future::Future;
use std::pin::Pin;

use cu_format::config::VaultConfig;
use cu_format::manifest_v2::{Acceptance, EntryKey, Node, Op, State};
use cu_vfs::{DirRef, NodeKind, VaultFs, VfsError};
use futures_util::StreamExt as _;
use futures_util::stream;

use cu_msg::Msg;

use crate::error::SyncError;

const CONCURRENCY: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryIssue {
    pub path: String,
    pub problem: String,
}

#[derive(Debug, Clone, Default)]
pub struct RegistryReport {
    pub files_ok: u64,
    pub dirs_ok: u64,
    pub unregistered: Vec<(String, EntryKey, Node)>,
    pub mismatches: Vec<RegistryIssue>,
    pub missing: Vec<RegistryIssue>,
    pub unresolved_pending: u64,
    pub confirmable: Vec<Op>,
}

impl RegistryReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty() && self.missing.is_empty()
    }
}

pub async fn verify_registry(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    state: &State,
    root: [u8; 32],
) -> Result<RegistryReport, SyncError> {
    let mut report = RegistryReport::default();
    let root_dir = vault_fs.resolve_dir_ref_untracked(config, "").await?;
    walk(vault_fs, config, state, &root_dir, root, "", &mut report).await?;
    Ok(report)
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}/{name}")
    }
}

type WalkFuture<'a> = Pin<Box<dyn Future<Output = Result<(), SyncError>> + Send + 'a>>;

fn walk<'a>(
    vault_fs: &'a VaultFs,
    config: &'a VaultConfig,
    state: &'a State,
    dir: &'a DirRef,
    dir_id: [u8; 32],
    path: &'a str,
    report: &'a mut RegistryReport,
) -> WalkFuture<'a> {
    Box::pin(async move {
        let entries = vault_fs.list_in(config, dir).await?;
        let listed: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();

        let files: Vec<String> = entries
            .iter()
            .filter(|e| e.kind == NodeKind::File)
            .map(|e| e.name.clone())
            .collect();
        let mut nonces: Vec<(String, Result<[u8; 12], VfsError>)> = stream::iter(files)
            .map(|name| async move {
                let nonce = vault_fs.read_header_nonce_in(config, dir, &name).await;
                (name, nonce)
            })
            .buffer_unordered(CONCURRENCY)
            .collect()
            .await;
        nonces.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, nonce) in nonces {
            let child_path = join(path, &name);
            let nonce = match nonce {
                Ok(n) => n,
                Err(VfsError::NotFound) => continue,
                Err(VfsError::Format(e)) => {
                    report.mismatches.push(RegistryIssue {
                        path: child_path,
                        problem: Msg::new("REGISTRY_HEADER_UNREADABLE").arg(&e).wire(),
                    });
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let key = EntryKey {
                parent: dir_id,
                name,
            };
            judge(state, &key, Node::File { nonce }, child_path, report);
        }

        for entry in entries.iter().filter(|e| e.kind == NodeKind::Dir) {
            let child_path = join(path, &entry.name);
            if child_path == ".cryptoupload/manifests" {
                continue;
            }
            let child = match vault_fs
                .descend_dir_ref_untracked(config, dir, &entry.name)
                .await
            {
                Ok(c) => c,
                Err(VfsError::NotFound) => continue,
                Err(e) => return Err(e.into()),
            };
            let key = EntryKey {
                parent: dir_id,
                name: entry.name.clone(),
            };
            let served = Node::Dir {
                dir_id: child.dir_id(),
            };
            let before = report.mismatches.len();
            judge(state, &key, served, child_path.clone(), report);
            if report.mismatches.len() == before {
                walk(
                    vault_fs,
                    config,
                    state,
                    &child,
                    child.dir_id(),
                    &child_path,
                    report,
                )
                .await?;
            }
        }

        report_unlisted(state, dir_id, &listed, path, report);
        Ok(())
    })
}

fn report_unlisted(
    state: &State,
    dir_id: [u8; 32],
    listed: &HashSet<&str>,
    path: &str,
    report: &mut RegistryReport,
) {
    let known: BTreeSet<&EntryKey> = state
        .slots
        .range(
            EntryKey {
                parent: dir_id,
                name: String::new(),
            }..,
        )
        .map(|(k, _)| k)
        .take_while(|k| k.parent == dir_id)
        .collect();
    for key in known {
        let slot = &state.slots[key];
        if slot.current.is_some() && !listed.contains(key.name.as_str()) {
            if state.judge(key, None) == Acceptance::Pending {
                report.unresolved_pending += 1;
                continue;
            }
            report.missing.push(RegistryIssue {
                path: join(path, &key.name),
                problem: Msg::new("REGISTRY_MISSING").wire(),
            });
        }
    }
}

fn judge(state: &State, key: &EntryKey, served: Node, path: String, report: &mut RegistryReport) {
    let is_dir = matches!(served, Node::Dir { .. });
    match state.judge(key, Some(&served)) {
        Acceptance::Current => {
            if is_dir {
                report.dirs_ok += 1;
            } else {
                report.files_ok += 1;
            }
            if state.slots.get(key).is_some_and(|s| !s.pending.is_empty()) {
                report.confirmable.push(Op::Confirm {
                    key: key.clone(),
                    node: Some(served),
                });
            }
        }
        Acceptance::Pending => report.unresolved_pending += 1,
        Acceptance::Unknown => report.unregistered.push((path, key.clone(), served)),
        Acceptance::Mismatch => report.mismatches.push(RegistryIssue {
            path,
            problem: Msg::new("REGISTRY_MISMATCH").wire(),
        }),
    }
}
