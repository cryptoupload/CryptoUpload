// SPDX-License-Identifier: GPL-3.0-or-later

use tauri::State;

use crate::error::UiError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryIssueDto {
    path: String,
    problem: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryVerifyDto {
    files_ok: u64,
    dirs_ok: u64,
    unregistered: usize,
    mismatches: Vec<RegistryIssueDto>,
    missing: Vec<RegistryIssueDto>,
    unresolved_pending: u64,
    registered: u64,
    sealed: bool,
    clean: bool,
}

#[tauri::command]
pub async fn vault_registry_verify(
    state: State<'_, AppState>,
    register: bool,
    seal: bool,
) -> Result<RegistryVerifyDto, UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    let verification = open_vault
        .vault
        .verify_registry(cu_core::RegistryVerifyOptions { register, seal })
        .await?;
    Ok(verification_dto(&verification))
}

#[tauri::command]
pub async fn vault_registry_reset(
    state: State<'_, AppState>,
) -> Result<RegistryVerifyDto, UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    open_vault.vault.reset_registry().await?;
    let verification = open_vault
        .vault
        .verify_registry(cu_core::RegistryVerifyOptions {
            register: true,
            seal: false,
        })
        .await?;
    Ok(verification_dto(&verification))
}

fn verification_dto(verification: &cu_core::RegistryVerification) -> RegistryVerifyDto {
    let report = &verification.report;
    let issues = |items: &[cu_core::RegistryIssue]| {
        items
            .iter()
            .map(|i| RegistryIssueDto {
                path: i.path.clone(),
                problem: i.problem.clone(),
            })
            .collect::<Vec<_>>()
    };
    RegistryVerifyDto {
        files_ok: report.files_ok,
        dirs_ok: report.dirs_ok,
        unregistered: report.unregistered.len(),
        mismatches: issues(&report.mismatches),
        missing: issues(&report.missing),
        unresolved_pending: report.unresolved_pending,
        registered: verification.registered,
        sealed: verification.sealed,
        clean: report.is_clean(),
    }
}
