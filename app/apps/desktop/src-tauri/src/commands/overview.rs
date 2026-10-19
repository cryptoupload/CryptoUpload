// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use tauri::State;

use crate::error::UiError;
use crate::state::{AppState, VaultLocation};

pub(crate) fn vault_ref_of(location: &VaultLocation) -> cu_core::VaultRef {
    match location {
        VaultLocation::Local(p) => cu_core::VaultRef::Local(p.clone()),
        VaultLocation::Remote(n) => cu_core::VaultRef::Remote(n.clone()),
    }
}

pub(crate) async fn activity_log_of_open_vault(
    state: &AppState,
) -> Option<cu_core::activity::ActivityLog> {
    let guard = state.vault.lock().await;
    let location = &guard.as_ref()?.location;
    let vault_ref = vault_ref_of(location);
    Some(cu_core::activity::ActivityLog::for_vault(
        &state.data_dir,
        &vault_ref,
        &state.local_state_for(&vault_ref),
    ))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDto {
    #[serde(flatten)]
    record: cu_core::activity::Record,
    warning: bool,
}

#[tauri::command]
pub async fn activity_recent(
    state: State<'_, AppState>,
    vault_root: String,
    limit: usize,
) -> Result<Vec<ActivityDto>, UiError> {
    let vault = cu_core::overview::vault_ref_from_location(&vault_root);
    Ok(cu_core::activity::ActivityLog::for_vault(
        &state.data_dir,
        &vault,
        &state.local_state_for(&vault),
    )
    .recent(limit.min(500))
    .into_iter()
    .map(|record| ActivityDto {
        warning: record.activity.is_warning(),
        record,
    })
    .collect())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFileDto {
    job: String,
    path: String,
    size: u64,
    modified: i64,
}

#[tauri::command]
pub async fn vault_recent(
    state: State<'_, AppState>,
    vault_root: String,
    limit: u32,
) -> Result<Vec<RecentFileDto>, UiError> {
    let vault = cu_core::overview::vault_ref_from_location(&vault_root);
    Ok(cu_core::overview::recent_files(
        &state.data_dir,
        &vault,
        limit.min(100),
        &state.local_state_for(&vault),
    )?
    .into_iter()
    .map(|f| RecentFileDto {
        job: f.job,
        path: f.path,
        size: f.size,
        modified: f.modified,
    })
    .collect())
}

#[tauri::command]
pub async fn vault_sync_states(
    state: State<'_, AppState>,
    vault_root: String,
    dir: String,
    names: Vec<String>,
) -> Result<HashMap<String, &'static str>, UiError> {
    let vault = cu_core::overview::vault_ref_from_location(&vault_root);
    Ok(cu_core::overview::sync_states(
        &state.data_dir,
        &vault,
        &dir,
        &names,
        &state.local_state_for(&vault),
    )?
    .into_iter()
    .map(|(name, s)| {
        (
            name,
            match s {
                cu_core::overview::EntrySyncState::Synced => "synced",
                cu_core::overview::EntrySyncState::Pending => "pending",
                cu_core::overview::EntrySyncState::Conflict => "conflict",
            },
        )
    })
    .collect())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceJobStatusDto {
    job: String,
    service_active: bool,
    next_run: Option<i64>,
    watcher_health: String,
}

#[tauri::command]
pub async fn jobs_service_status(
    state: State<'_, AppState>,
    vault_root: String,
) -> Result<Vec<ServiceJobStatusDto>, UiError> {
    let vault = cu_core::overview::vault_ref_from_location(&vault_root);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Ok(
        cu_core::overview::service_status(&state.data_dir, &vault, now_ms)?
            .into_iter()
            .map(|s| ServiceJobStatusDto {
                job: s.job,
                service_active: s.service_active,
                next_run: s.next_run,
                watcher_health: s.watcher_health,
            })
            .collect(),
    )
}

#[tauri::command]
pub async fn job_cancel(state: State<'_, AppState>, name: String) -> Result<bool, UiError> {
    let running = state
        .running_jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(running.get(&name).is_some_and(|token| {
        token.cancel();
        true
    }))
}
