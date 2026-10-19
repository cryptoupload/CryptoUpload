// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use tauri::AppHandle;
use tauri::State;

use crate::error::UiError;
use crate::state::{AppState, VaultLocation};

fn to_vault_ref(location: &VaultLocation) -> cu_core::VaultRef {
    match location {
        VaultLocation::Local(p) => cu_core::VaultRef::Local(p.clone()),
        VaultLocation::Remote(name) => cu_core::VaultRef::Remote(name.clone()),
    }
}

#[tauri::command]
pub async fn recovery_kit_generate(
    app: AppHandle,
    state: State<'_, AppState>,
    labels: std::collections::HashMap<String, String>,
) -> Result<String, UiError> {
    render(&app, &state, labels).await
}

async fn render(
    app: &AppHandle,
    state: &AppState,
    labels: std::collections::HashMap<String, String>,
) -> Result<String, UiError> {
    let text = cu_core::recovery_kit::KitText::new(labels);
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    let location = to_vault_ref(&open_vault.location);

    let job_specs = crate::commands::jobs::store(state)
        .load()
        .map_err(|e| UiError::from_msg(&e))?;

    let remote_details: std::collections::HashMap<String, cu_core::RemoteConfig> = {
        let unlocked = state.unlocked_remotes.lock().await;
        unlocked
            .iter()
            .map(|(name, entry)| (name.clone(), entry.config.clone()))
            .collect()
    };

    let app_version = app.package_info().version.to_string();
    let kit = cu_core::recovery_kit::build(
        &open_vault.vault,
        &location,
        &job_specs,
        &remote_details,
        &app_version,
        &text,
    );
    Ok(cu_core::recovery_kit::render_html(&kit, &text))
}

#[tauri::command]
pub async fn recovery_kit_save(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    labels: std::collections::HashMap<String, String>,
) -> Result<(), UiError> {
    let html = render(&app, &state, labels).await?;
    std::fs::write(&path, html)
        .map_err(|e| UiError::coded(&Msg::new("IO_AT_PATH").arg(&path).arg(&e)))
}
