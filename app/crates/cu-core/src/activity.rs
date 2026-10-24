// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Coded;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use cu_jobs::VaultRef;

use crate::local_state::LocalStateAccess;

pub const MAX_BYTES: u64 = 256 * 1024;
pub const KEEP_LINES: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Activity {
    RunFinished {
        job: String,
        uploaded: u64,
        deleted: u64,
        renamed: u64,
        bytes: u64,
        conflicts: u64,
    },
    RunFailed {
        job: String,
        message: String,
    },
    RunCancelled {
        job: String,
    },
    ConflictKeptBoth {
        job: String,
        path: String,
        detail: String,
    },
    AnomalyBlocked {
        job: String,
        reason: String,
    },
    VerifyPassed {
        job: String,
        files: u64,
    },
    VerifyIssues {
        job: String,
        issues: u64,
    },
    Imported {
        path: String,
        files: u64,
    },
    TrashEmptied {
        items: u64,
    },
}

impl Activity {
    #[must_use]
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            Self::RunFailed { .. }
                | Self::ConflictKeptBoth { .. }
                | Self::AnomalyBlocked { .. }
                | Self::VerifyIssues { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub at: i64,
    #[serde(flatten)]
    pub activity: Activity,
}

#[derive(Debug, Clone)]
pub struct ActivityLog {
    path: PathBuf,
    access: LocalStateAccess,
}

const SEALED_PREFIX: &str = "enc1:";

impl ActivityLog {
    #[must_use]
    pub fn for_vault(data_dir: &Path, vault: &VaultRef, access: &LocalStateAccess) -> Self {
        let log = Self {
            path: Self::path_for(data_dir, vault),
            access: access.clone(),
        };
        let _ = log.align();
        log
    }

    pub(crate) fn path_for(data_dir: &Path, vault: &VaultRef) -> PathBuf {
        let mut hasher = cu_crypto::hash::Hasher::new();
        match vault {
            VaultRef::Local(root) => {
                let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
                hasher.update(b"local\0");
                hasher.update(canonical.to_string_lossy().as_bytes());
            }
            VaultRef::Remote(name) => {
                hasher.update(b"remote\0");
                hasher.update(name.as_bytes());
            }
        }
        let digest = hasher.finalize();
        let name = digest[..16]
            .iter()
            .fold(String::with_capacity(32), |mut acc, b| {
                use std::fmt::Write as _;
                let _ = write!(acc, "{b:02x}");
                acc
            });
        data_dir
            .join("cryptoupload")
            .join("activity")
            .join(format!("{name}.jsonl"))
    }

    fn align(&self) -> std::io::Result<()> {
        if !self.access.writable {
            return Ok(());
        }
        if self.access.log_disabled {
            return match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        let Some(cipher) = &self.access.cipher else {
            return Ok(());
        };
        let Ok(content) = std::fs::read_to_string(&self.path) else {
            return Ok(());
        };
        let want_sealed = self.access.encrypt;
        if content
            .lines()
            .all(|line| line.starts_with(SEALED_PREFIX) == want_sealed)
        {
            return Ok(());
        }
        let mut out = String::with_capacity(content.len());
        for line in content.lines() {
            let converted = match (line.strip_prefix(SEALED_PREFIX), want_sealed) {
                (Some(_), true) | (None, false) => Some(line.to_string()),
                (None, true) => match cipher.seal(line) {
                    Some(sealed) => Some(format!("{SEALED_PREFIX}{sealed}")),
                    None => return Err(std::io::Error::other("INDEX_SEAL_FAILED")),
                },
                (Some(sealed), false) => cipher.open(sealed),
            };
            if let Some(converted) = converted {
                out.push_str(&converted);
                out.push('\n');
            }
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        owner_only(
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true),
        )
        .open(&tmp)?
        .write_all(out.as_bytes())?;
        std::fs::rename(&tmp, &self.path)
    }

    pub fn clear_all(data_dir: &Path) {
        let _ = std::fs::remove_dir_all(data_dir.join("cryptoupload").join("activity"));
    }

    pub fn record(&self, activity: Activity) {
        let _ = self.try_record(now_unix(), activity);
    }

    fn try_record(&self, at: i64, activity: Activity) -> std::io::Result<()> {
        if !self.access.writable || self.access.log_disabled {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(&Record { at, activity })?;
        let mut line = match (&self.access.cipher, self.access.encrypt) {
            (Some(cipher), true) => match cipher.seal(&json) {
                Some(sealed) => format!("{SEALED_PREFIX}{sealed}").into_bytes(),
                None => return Err(std::io::Error::other("INDEX_SEAL_FAILED")),
            },
            (None, true) => return Ok(()),
            (_, false) => json.into_bytes(),
        };
        line.push(b'\n');
        let mut file =
            owner_only(std::fs::OpenOptions::new().create(true).append(true)).open(&self.path)?;
        file.write_all(&line)?;
        if file.metadata()?.len() > MAX_BYTES {
            drop(file);
            self.compact()?;
        }
        Ok(())
    }

    fn compact(&self) -> std::io::Result<()> {
        let content = std::fs::read_to_string(&self.path)?;
        let lines: Vec<&str> = content.lines().collect();
        let keep = &lines[lines.len().saturating_sub(KEEP_LINES)..];
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut out = keep.join("\n");
        out.push('\n');
        owner_only(
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true),
        )
        .open(&tmp)?
        .write_all(out.as_bytes())?;
        std::fs::rename(&tmp, &self.path)
    }

    #[must_use]
    pub fn recent(&self, limit: usize) -> Vec<Record> {
        let Ok(content) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        content
            .lines()
            .rev()
            .filter_map(|line| match line.strip_prefix(SEALED_PREFIX) {
                Some(sealed) => {
                    let json = self.access.cipher.as_ref()?.open(sealed)?;
                    serde_json::from_str::<Record>(&json).ok()
                }
                None => serde_json::from_str::<Record>(line).ok(),
            })
            .take(limit)
            .collect()
    }

    pub fn record_run(&self, job: &str, result: &Result<cu_sync::SyncReport, crate::CoreError>) {
        match result {
            Ok(report) => {
                for c in &report.conflicts {
                    let detail = match &c.resolution {
                        cu_sync::model::ConflictResolution::KeptBoth { sidecar_path } => {
                            format!(
                                "{}{}{}",
                                c.reason,
                                cu_msg::LIST_SEP,
                                cu_msg::Msg::new("CONFLICT_KEPT_BOTH")
                                    .arg(sidecar_path)
                                    .wire()
                            )
                        }
                        cu_sync::model::ConflictResolution::Resurrected => format!(
                            "{}{}{}",
                            c.reason,
                            cu_msg::LIST_SEP,
                            cu_msg::Msg::new("CONFLICT_RESURRECTED").wire()
                        ),
                    };
                    self.record(Activity::ConflictKeptBoth {
                        job: job.to_string(),
                        path: c.vault_path.clone(),
                        detail,
                    });
                }
                self.record(Activity::RunFinished {
                    job: job.to_string(),
                    uploaded: report.stats.uploads,
                    deleted: report.stats.deletes,
                    renamed: report.stats.renames,
                    bytes: report.stats.bytes_transferred,
                    conflicts: report.conflicts.len() as u64,
                });
            }
            Err(crate::CoreError::AnomalyDetected { reason, .. }) => {
                self.record(Activity::AnomalyBlocked {
                    job: job.to_string(),
                    reason: cu_msg::wire_list(reason),
                });
            }
            Err(e) if e.is_cancelled() => {
                self.record(Activity::RunCancelled {
                    job: job.to_string(),
                });
            }
            Err(e) => self.record(Activity::RunFailed {
                job: job.to_string(),
                message: e.msg().wire(),
            }),
        }
    }
}

fn owner_only(options: &mut std::fs::OpenOptions) -> &mut std::fs::OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
