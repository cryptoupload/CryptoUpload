// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::PathBuf;

use tauri::State;

use crate::error::UiError;
use crate::state::AppState;

fn decode_inbox_public_key(raw: &str) -> Result<[u8; 32], UiError> {
    let bytes = data_encoding::BASE64URL_NOPAD
        .decode(raw.trim().as_bytes())
        .map_err(|_| UiError::coded(&Msg::new("PUBKEY_NOT_BASE64")))?;
    bytes
        .try_into()
        .map_err(|_| UiError::coded(&Msg::new("PUBKEY_NOT_32_BYTES")))
}

#[tauri::command]
pub async fn inbox_deposit(
    state: State<'_, AppState>,
    remote_name: String,
    pubkey_b64: String,
    file_path: PathBuf,
    label: String,
) -> Result<(), UiError> {
    let public_key = decode_inbox_public_key(&pubkey_b64)?;
    let (cfg, state_db, refresh_sink) = {
        let unlocked = state.unlocked_remotes.lock().await;
        let entry = unlocked
            .get(&remote_name)
            .ok_or_else(|| UiError::coded(&Msg::new("REMOTE_NOT_UNLOCKED").arg(&remote_name)))?;
        let cfg = entry.config.clone();
        let state_db = cu_core::resume::remote_state_db_path(
            &state.data_dir,
            &cu_core::remote::remote_id(&cfg),
        );
        let refresh_sink = cu_core::remote::persisting_refresh_sink(
            state.secrets_file_path(),
            remote_name.clone(),
            entry.password.as_bytes().to_vec(),
        );
        (cfg, state_db, refresh_sink)
    };
    cu_core::deposit_via_remote(
        &cfg,
        &state_db,
        &public_key,
        &file_path,
        &label,
        Some(refresh_sink),
    )
    .await?;
    Ok(())
}
