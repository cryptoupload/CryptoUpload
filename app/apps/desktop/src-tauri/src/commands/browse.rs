// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::PathBuf;

use tauri::State;
use tauri::ipc::Channel;

use crate::error::UiError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryDto {
    name: String,
    kind: EntryKind,
    size: Option<u64>,
    modified: Option<i64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailedEntryDto {
    name: String,
    encrypted_name: String,
    kind: EntryKind,
}

impl From<cu_core::DetailedEntry> for DetailedEntryDto {
    fn from(entry: cu_core::DetailedEntry) -> Self {
        Self {
            name: entry.name,
            encrypted_name: entry.encrypted_name,
            kind: match entry.kind {
                cu_core::NodeKind::File => EntryKind::File,
                cu_core::NodeKind::Dir => EntryKind::Dir,
            },
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
}

impl From<cu_core::Entry> for EntryDto {
    fn from(entry: cu_core::Entry) -> Self {
        Self {
            name: entry.name,
            kind: match entry.kind {
                cu_core::NodeKind::File => EntryKind::File,
                cu_core::NodeKind::Dir => EntryKind::Dir,
            },
            size: entry.size,
            modified: entry.modified,
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TransferEvent {
    File { vault_path: String },
    Bytes { count: u64 },
    Item { index: u64 },
}

impl From<cu_core::ProgressEvent> for TransferEvent {
    fn from(event: cu_core::ProgressEvent) -> Self {
        match event {
            cu_core::ProgressEvent::File { vault_path } => Self::File { vault_path },
            cu_core::ProgressEvent::Bytes { count } => Self::Bytes { count },
        }
    }
}

#[tauri::command]
pub async fn vault_list(
    state: State<'_, AppState>,
    vault_dir: String,
) -> Result<Vec<EntryDto>, UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    let mut entries: Vec<EntryDto> = vault
        .list(&vault_dir)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundEntryDto {
    path: String,
    is_dir: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindOutcomeDto {
    truncated: bool,
    cancelled: bool,
}

const FIND_STREAM_LIMIT: usize = 5_000;

#[tauri::command]
pub async fn vault_find_stream(
    state: State<'_, AppState>,
    start_dir: String,
    pattern: String,
    on_result: Channel<FoundEntryDto>,
) -> Result<FindOutcomeDto, UiError> {
    let token = cu_core::CancelToken::new();
    {
        let mut slot = state
            .find_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = slot.replace(token.clone()) {
            previous.cancel();
        }
    }
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    let outcome = vault
        .find_streaming(
            &start_dir,
            &pattern,
            FIND_STREAM_LIMIT,
            &token,
            &mut |found| {
                let _ = on_result.send(FoundEntryDto {
                    path: found.path,
                    is_dir: found.is_dir,
                });
            },
        )
        .await?;
    Ok(FindOutcomeDto {
        truncated: outcome.truncated,
        cancelled: outcome.cancelled,
    })
}

#[tauri::command]
pub async fn vault_find_cancel(state: State<'_, AppState>) -> Result<(), UiError> {
    let slot = state
        .find_cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(token) = slot.as_ref() {
        token.cancel();
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevealedListingDto {
    physical_dir: String,
    entries: Vec<DetailedEntryDto>,
}

#[tauri::command]
pub async fn vault_list_revealed(
    state: State<'_, AppState>,
    vault_dir: String,
) -> Result<RevealedListingDto, UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    let mut entries: Vec<DetailedEntryDto> = vault
        .list_detailed(&vault_dir)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let physical_dir = vault.physical_dir(&vault_dir).await?;
    Ok(RevealedListingDto {
        physical_dir,
        entries,
    })
}

#[tauri::command]
pub async fn vault_mkdir(state: State<'_, AppState>, vault_path: String) -> Result<(), UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    vault.mkdir(&vault_path).await?;
    Ok(())
}

#[tauri::command]
pub async fn local_path_kinds(paths: Vec<PathBuf>) -> Result<Vec<bool>, UiError> {
    let mut kinds = Vec::with_capacity(paths.len());
    for path in paths {
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|e| UiError::coded(&Msg::new("IO_AT_PATH").arg(path.display()).arg(&e)))?;
        kinds.push(meta.is_dir());
    }
    Ok(kinds)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    pub local_src: PathBuf,
    pub vault_dest: String,
}

#[tauri::command]
pub async fn vault_import_many(
    state: State<'_, AppState>,
    items: Vec<ImportItem>,
    on_progress: Channel<TransferEvent>,
) -> Result<Vec<Option<String>>, UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    let pairs: Vec<(PathBuf, String)> = items
        .iter()
        .map(|item| (item.local_src.clone(), item.vault_dest.clone()))
        .collect();
    let mut files = vec![0u64; pairs.len()];
    let mut last_index: Option<usize> = None;
    let mut on_event = |index: usize, event: cu_core::ProgressEvent| {
        if last_index != Some(index) {
            last_index = Some(index);
            let _ = on_progress.send(TransferEvent::Item {
                index: index as u64,
            });
        }
        if matches!(event, cu_core::ProgressEvent::File { .. }) {
            files[index] += 1;
        }
        let _ = on_progress.send(event.into());
    };
    let results = vault.import_many(&pairs, &mut on_event).await;
    drop(guard);
    if let Some(log) = crate::commands::overview::activity_log_of_open_vault(&state).await {
        for ((item, result), count) in items.iter().zip(&results).zip(&files) {
            if result.is_ok() {
                log.record(cu_core::activity::Activity::Imported {
                    path: item.vault_dest.clone(),
                    files: *count,
                });
            }
        }
    }
    Ok(results
        .into_iter()
        .map(|result| result.err().map(|e| e.to_string()))
        .collect())
}

#[tauri::command]
pub async fn vault_export(
    state: State<'_, AppState>,
    vault_src: String,
    local_dest: PathBuf,
    on_progress: Channel<TransferEvent>,
) -> Result<(), UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    let mut on_event = |event: cu_core::ProgressEvent| {
        let _ = on_progress.send(event.into());
    };
    vault
        .export_tree(&vault_src, &local_dest, &mut on_event)
        .await?;
    Ok(())
}
