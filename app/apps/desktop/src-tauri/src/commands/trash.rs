// SPDX-License-Identifier: GPL-3.0-or-later

use tauri::State;

use crate::error::UiError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrashKindDto {
    File,
    Dir,
}

impl From<cu_core::TrashKind> for TrashKindDto {
    fn from(kind: cu_core::TrashKind) -> Self {
        match kind {
            cu_core::TrashKind::File => Self::File,
            cu_core::TrashKind::Dir => Self::Dir,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntryDto {
    id: String,
    original_path: Option<String>,
    name: String,
    kind: TrashKindDto,
    deleted_at: u64,
}

impl From<cu_core::TrashEntry> for TrashEntryDto {
    fn from(entry: cu_core::TrashEntry) -> Self {
        Self {
            id: entry.id,
            original_path: entry.original_path,
            name: entry.name,
            kind: entry.kind.into(),
            deleted_at: entry.deleted_at,
        }
    }
}

#[tauri::command]
pub async fn vault_trash(state: State<'_, AppState>, vault_path: String) -> Result<(), UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    vault.trash(&vault_path).await?;
    Ok(())
}

#[tauri::command]
pub async fn trash_list(state: State<'_, AppState>) -> Result<Vec<TrashEntryDto>, UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    Ok(vault
        .list_trash()
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub async fn trash_restore(state: State<'_, AppState>, id: String) -> Result<String, UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    Ok(vault.restore_from_trash(&id).await?)
}

#[tauri::command]
pub async fn trash_purge(state: State<'_, AppState>, id: String) -> Result<(), UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    vault.purge_trash(&id).await?;
    Ok(())
}

#[tauri::command]
pub async fn trash_empty(state: State<'_, AppState>) -> Result<(), UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    let items = vault.list_trash().await.map_or(0, |t| t.len() as u64);
    vault.empty_trash().await?;
    drop(guard);
    if let Some(log) = crate::commands::overview::activity_log_of_open_vault(&state).await {
        log.record(cu_core::activity::Activity::TrashEmptied { items });
    }
    Ok(())
}
