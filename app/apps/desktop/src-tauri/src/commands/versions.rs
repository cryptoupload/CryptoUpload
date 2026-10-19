// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::Path;

use tauri::State;

use crate::commands::jobs::{store, vault_ref_matches};
use crate::error::UiError;
use crate::state::{AppState, OpenVault, VaultLocation};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDto {
    version: i64,
    created_at: i64,
    size: u64,
    name: String,
    deleted: bool,
    location: &'static str,
}

impl From<cu_core::VersionRow> for VersionDto {
    fn from(row: cu_core::VersionRow) -> Self {
        Self {
            version: row.version,
            created_at: row.created_at,
            size: row.size,
            name: row.name,
            deleted: row.deleted,
            location: match row.location {
                cu_core::VersionLocation::Canonical => "canonical",
                cu_core::VersionLocation::Archived => "archived",
            },
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultVersionsResult {
    no_job_configured: bool,
    versions: Vec<VersionDto>,
}

fn find_job_for_path(
    jobs: Vec<cu_core::JobSpec>,
    location: &VaultLocation,
    full_vault_path: &str,
) -> Option<(cu_core::JobSpec, String)> {
    let full = full_vault_path.trim_matches('/');
    jobs.into_iter()
        .filter(|spec| vault_ref_matches(&spec.vault, location))
        .filter_map(|spec| {
            let prefix_len = spec.vault_path.trim_matches('/').len();
            let rel = relative_to_job_root(full, &spec.vault_path)?;
            Some((prefix_len, spec, rel))
        })
        .max_by_key(|(prefix_len, ..)| *prefix_len)
        .map(|(_, spec, rel)| (spec, rel))
}

fn relative_to_job_root(full: &str, job_vault_path: &str) -> Option<String> {
    let prefix = job_vault_path.trim_matches('/');
    if prefix.is_empty() {
        return Some(full.to_string());
    }
    if full == prefix {
        return Some(String::new());
    }
    full.strip_prefix(prefix)?
        .strip_prefix('/')
        .map(str::to_string)
}

#[tauri::command]
pub async fn vault_versions(
    state: State<'_, AppState>,
    path: String,
) -> Result<VaultVersionsResult, UiError> {
    let (open_vault, generation) = state.take_vault().await.ok_or_else(UiError::no_vault)?;

    let jobs = match store(&state).load() {
        Ok(jobs) => jobs,
        Err(e) => {
            state.restore_vault(open_vault, generation).await;
            return Err(UiError::from_msg(&e));
        }
    };

    let Some((spec, rel_path)) = find_job_for_path(jobs, &open_vault.location, &path) else {
        state.restore_vault(open_vault, generation).await;
        return Ok(VaultVersionsResult {
            no_job_configured: true,
            versions: Vec::new(),
        });
    };

    let state_db = match cu_core::job_state_db_path(&state.data_dir, &spec) {
        Ok(path) => path,
        Err(e) => {
            state.restore_vault(open_vault, generation).await;
            return Err(UiError::from(e));
        }
    };
    if let Some(parent) = state_db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let job = cu_core::SyncJob::open(
        open_vault.vault,
        &state_db,
        &spec.local_root,
        &spec.vault_path,
        &state.data_dir,
    )?;
    let result = job.list_versions(&rel_path);
    let vault = job.into_vault();
    state
        .restore_vault(
            OpenVault {
                vault,
                location: open_vault.location,
            },
            generation,
        )
        .await;

    Ok(VaultVersionsResult {
        no_job_configured: false,
        versions: result?.into_iter().map(VersionDto::from).collect(),
    })
}

#[tauri::command]
pub async fn vault_restore_version(
    state: State<'_, AppState>,
    path: String,
    version: i64,
    dest: String,
) -> Result<(), UiError> {
    let (open_vault, generation) = state.take_vault().await.ok_or_else(UiError::no_vault)?;

    let jobs = match store(&state).load() {
        Ok(jobs) => jobs,
        Err(e) => {
            state.restore_vault(open_vault, generation).await;
            return Err(UiError::from_msg(&e));
        }
    };

    let Some((spec, rel_path)) = find_job_for_path(jobs, &open_vault.location, &path) else {
        state.restore_vault(open_vault, generation).await;
        return Err(UiError::coded(&Msg::new("VERSIONS_NO_JOB")));
    };

    let state_db = match cu_core::job_state_db_path(&state.data_dir, &spec) {
        Ok(path) => path,
        Err(e) => {
            state.restore_vault(open_vault, generation).await;
            return Err(UiError::from(e));
        }
    };
    if let Some(parent) = state_db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut job = cu_core::SyncJob::open(
        open_vault.vault,
        &state_db,
        &spec.local_root,
        &spec.vault_path,
        &state.data_dir,
    )?;
    let result = job
        .restore_version(&rel_path, version, Path::new(&dest))
        .await;
    let vault = job.into_vault();
    state
        .restore_vault(
            OpenVault {
                vault,
                location: open_vault.location,
            },
            generation,
        )
        .await;

    result.map_err(UiError::from)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyIssueDto {
    vault_path: String,
    problem: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobVerifyResult {
    level: u8,
    issues: Vec<VerifyIssueDto>,
}

#[tauri::command]
pub async fn job_verify(
    state: State<'_, AppState>,
    name: String,
    level: u8,
) -> Result<JobVerifyResult, UiError> {
    if level != 1 && level != 2 {
        return Err(UiError::coded(&Msg::new("VERIFY_LEVEL_UNKNOWN")));
    }
    let (open_vault, generation) = state.take_vault().await.ok_or_else(UiError::no_vault)?;

    let jobs = match store(&state).load() {
        Ok(jobs) => jobs,
        Err(e) => {
            state.restore_vault(open_vault, generation).await;
            return Err(UiError::from_msg(&e));
        }
    };
    let Some(spec) = jobs.into_iter().find(|j| j.name == name) else {
        state.restore_vault(open_vault, generation).await;
        return Err(UiError::coded(&Msg::new("JOB_NOT_FOUND").arg(&name)));
    };
    if !vault_ref_matches(&spec.vault, &open_vault.location) {
        state.restore_vault(open_vault, generation).await;
        return Err(UiError::coded(&Msg::new("VERIFY_JOB_OTHER_VAULT")));
    }

    let state_db = match cu_core::job_state_db_path(&state.data_dir, &spec) {
        Ok(path) => path,
        Err(e) => {
            state.restore_vault(open_vault, generation).await;
            return Err(UiError::from(e));
        }
    };
    if let Some(parent) = state_db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let job = cu_core::SyncJob::open(
        open_vault.vault,
        &state_db,
        &spec.local_root,
        &spec.vault_path,
        &state.data_dir,
    )?;
    let runtime = tokio::runtime::Handle::current();
    let joined = tokio::task::spawn_blocking(move || {
        let result = runtime.block_on(async {
            if level == 1 {
                job.verify_existence().await
            } else {
                job.verify_content().await
            }
        });
        (job.into_vault(), result)
    })
    .await;
    let Ok((vault, result)) = joined else {
        return Err(UiError::coded(&Msg::new("VERIFY_INTERRUPTED")));
    };
    state
        .restore_vault(
            OpenVault {
                vault,
                location: open_vault.location,
            },
            generation,
        )
        .await;

    let result = result?;
    let log = state.activity_log_for(&spec.vault);
    if result.is_empty() {
        log.record(cu_core::activity::Activity::VerifyPassed {
            job: spec.name.clone(),
            files: cu_core::storage_usage(&state.data_dir)
                .ok()
                .and_then(|u| u.into_iter().find(|u| u.job_name == spec.name))
                .map_or(0, |u| u.file_count),
        });
    } else {
        log.record(cu_core::activity::Activity::VerifyIssues {
            job: spec.name.clone(),
            issues: result.len() as u64,
        });
    }
    Ok(JobVerifyResult {
        level,
        issues: result
            .into_iter()
            .map(|i| VerifyIssueDto {
                vault_path: i.vault_path,
                problem: i.problem,
            })
            .collect(),
    })
}
