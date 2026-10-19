// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::PathBuf;

use tauri::State;
use tauri::ipc::Channel;

use crate::commands::browse::TransferEvent;
use crate::error::UiError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfoDto {
    original_name: String,
    bytes: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedShareDto {
    path: String,
    original_name: String,
    bytes: u64,
}

#[tauri::command]
pub async fn vault_share_file(
    state: State<'_, AppState>,
    vault_src: String,
    out: PathBuf,
    password: String,
    overwrite: bool,
    on_progress: Channel<TransferEvent>,
) -> Result<ShareInfoDto, UiError> {
    let guard = state.vault.lock().await;
    let vault = &guard.as_ref().ok_or_else(UiError::no_vault)?.vault;
    let mut on_bytes = |count: u64| {
        let _ = on_progress.send(TransferEvent::Bytes { count });
    };
    let info = vault
        .share_file(
            &vault_src,
            &out,
            password.as_bytes(),
            overwrite,
            &mut on_bytes,
        )
        .await?;
    Ok(ShareInfoDto {
        original_name: info.original_name,
        bytes: info.bytes,
    })
}

#[tauri::command]
pub async fn open_share_package(
    package: PathBuf,
    password: String,
    dest_dir: PathBuf,
    on_progress: Channel<TransferEvent>,
) -> Result<OpenedShareDto, UiError> {
    let mut on_bytes = |count: u64| {
        let _ = on_progress.send(TransferEvent::Bytes { count });
    };
    let opened = cu_core::open_share(&package, password.as_bytes(), &dest_dir, &mut on_bytes)
        .await
        .map_err(|e| {
            if e.is_decrypt_failure() {
                UiError::coded(&Msg::new("SHARE_BAD_PASSWORD_OR_DAMAGED"))
            } else {
                UiError::from(e)
            }
        })?;
    Ok(OpenedShareDto {
        path: opened.path.to_string_lossy().into_owned(),
        original_name: opened.original_name,
        bytes: opened.bytes,
    })
}
