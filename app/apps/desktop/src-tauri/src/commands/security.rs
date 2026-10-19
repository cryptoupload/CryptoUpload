// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use tauri::State;
use zeroize::Zeroize;

use crate::commands::jobs::vault_ref_matches;
use crate::error::UiError;
use crate::guard::{self, ForceLockReport, LockReason};
use crate::settings::AppSecurity;
use crate::state::AppState;

#[derive(serde::Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Availability {
    Available,
    Unavailable,
}

#[derive(serde::Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OptionStatus {
    pub id: &'static str,
    pub availability: Availability,
    pub reason: Option<&'static str>,
    pub caveat: Option<&'static str>,
}

const fn available(id: &'static str) -> OptionStatus {
    OptionStatus {
        id,
        availability: Availability::Available,
        reason: None,
        caveat: None,
    }
}

#[cfg(not(target_os = "linux"))]
const fn available_with(id: &'static str, caveat: &'static str) -> OptionStatus {
    OptionStatus {
        id,
        availability: Availability::Available,
        reason: None,
        caveat: Some(caveat),
    }
}

const fn unavailable(id: &'static str, reason: &'static str) -> OptionStatus {
    OptionStatus {
        id,
        availability: Availability::Unavailable,
        reason: Some(reason),
        caveat: None,
    }
}

#[must_use]
pub fn option_statuses() -> Vec<OptionStatus> {
    #[cfg(any(target_os = "linux", windows))]
    let read_only_drive = available("readOnlyDrive");
    #[cfg(not(any(target_os = "linux", windows)))]
    let read_only_drive = unavailable("readOnlyDrive", "SEC_NO_DRIVE_YET");
    #[cfg(target_os = "linux")]
    let hide_from_capture = unavailable("hideFromCapture", "SEC_NO_CAPTURE_HIDING");
    #[cfg(not(target_os = "linux"))]
    let hide_from_capture = available("hideFromCapture");
    #[cfg(any(target_os = "linux", windows))]
    let block_remote_sessions = available("blockRemoteSessions");
    #[cfg(not(any(target_os = "linux", windows)))]
    let block_remote_sessions = available_with("blockRemoteSessions", "SEC_REMOTE_LIMITED");
    #[cfg(target_os = "linux")]
    let program_access = available("programAccess");
    #[cfg(target_os = "linux")]
    let decoys = available("decoys");
    #[cfg(windows)]
    let decoys = unavailable("decoys", "SEC_DECOYS_WINDOWS");
    #[cfg(not(any(target_os = "linux", windows)))]
    let decoys = unavailable("decoys", "SEC_NO_DRIVE_YET");
    #[cfg(windows)]
    let program_access = unavailable("programAccess", "SEC_PROGRAM_ACCESS_WINDOWS");
    #[cfg(not(any(target_os = "linux", windows)))]
    let program_access = unavailable("programAccess", "SEC_NO_DRIVE_YET");
    #[cfg(any(target_os = "linux", windows))]
    let lock_unmounts_drive = available("lockUnmountsDrive");
    #[cfg(not(any(target_os = "linux", windows)))]
    let lock_unmounts_drive = unavailable("lockUnmountsDrive", "SEC_NO_DRIVE_YET");
    #[cfg(target_os = "linux")]
    let lock_on_session_events = available("lockOnSessionEvents");
    #[cfg(not(target_os = "linux"))]
    let lock_on_session_events =
        available_with("lockOnSessionEvents", "SEC_SESSION_EVENTS_LIMITED");
    vec![
        lock_unmounts_drive,
        lock_on_session_events,
        block_remote_sessions,
        program_access,
        read_only_drive,
        hide_from_capture,
        available("noUnlockShortcuts"),
        available("encryptLocalState"),
        available("disableActivityLog"),
        available("lockMemory"),
        decoys,
        available("panic"),
        available("failedUnlockNotice"),
        available("clearRecentsOnExit"),
    ]
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityState {
    policy: cu_core::VaultPolicy,
    effective: cu_core::VaultPolicy,
    options: Vec<OptionStatus>,
    two_factor: bool,
}

fn state_of(vault: &cu_core::Vault) -> SecurityState {
    SecurityState {
        policy: vault.policy().clone(),
        effective: vault.effective_policy(),
        options: option_statuses(),
        two_factor: vault.has_two_factor(),
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PolicySetOutcome {
    #[serde(rename_all = "camelCase")]
    Saved {
        state: SecurityState,
        revoked_jobs: Vec<String>,
    },
    PasswordRequired,
}

#[derive(serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SecurityRuntime {
    lock_unmounts_drive: bool,
    lock_on_session_events: bool,
    max_idle_minutes: u32,
}

#[tauri::command]
pub async fn security_runtime(state: State<'_, AppState>) -> Result<SecurityRuntime, UiError> {
    let guard = state.vault.lock().await;
    Ok(guard
        .as_ref()
        .map_or_else(SecurityRuntime::default, |open| {
            let policy = open.vault.effective_policy();
            SecurityRuntime {
                lock_unmounts_drive: policy.lock_unmounts_drive,
                lock_on_session_events: policy.lock_on_session_events,
                max_idle_minutes: policy.max_idle_minutes,
            }
        }))
}

#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum AutoLockOutcome {
    Locked { report: ForceLockReport },
    SkippedMounted,
}

#[tauri::command]
pub async fn vault_auto_lock(state: State<'_, AppState>) -> Result<AutoLockOutcome, UiError> {
    #[cfg(any(target_os = "linux", windows))]
    {
        let mounted = state.mount.lock().await.is_some();
        let unmounts = state
            .vault
            .lock()
            .await
            .as_ref()
            .is_some_and(|open| open.vault.effective_policy().lock_unmounts_drive);
        if mounted && !unmounts {
            return Ok(AutoLockOutcome::SkippedMounted);
        }
    }
    let report = guard::force_lock(&state, LockReason::Idle, guard::FLUSH_BUDGET).await;
    Ok(AutoLockOutcome::Locked { report })
}

#[tauri::command]
pub async fn vault_force_lock(
    state: State<'_, AppState>,
    reason: LockReason,
) -> Result<ForceLockReport, UiError> {
    Ok(guard::force_lock(&state, reason, guard::FLUSH_BUDGET).await)
}

#[tauri::command]
pub async fn app_security_get(state: State<'_, AppState>) -> Result<AppSecurity, UiError> {
    Ok(state.settings.lock().await.security.clone())
}

#[tauri::command]
pub async fn app_security_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    security: AppSecurity,
) -> Result<AppSecurity, UiError> {
    let mut security = security;
    security.panic_shortcut = security
        .panic_shortcut
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    guard::set_panic_shortcut(&app, security.panic_shortcut.as_deref())
        .map_err(|m| UiError::coded(&m))?;
    guard::apply_capture_protection(&app, security.hide_from_capture);

    if security.lock_memory
        && let Some(open_vault) = state.vault.lock().await.as_mut()
    {
        state.lock_vault_memory(&mut open_vault.vault);
    }

    let mut settings = state.settings.lock().await;
    settings.security = security;
    if let Err(e) = crate::settings::save(&state.data_dir, &settings) {
        eprintln!("cannot save ui.json: {e}");
    }
    Ok(settings.security.clone())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryLockStatus {
    locked: usize,
    failed: usize,
}

#[tauri::command]
pub async fn memory_lock_status(
    state: State<'_, AppState>,
) -> Result<Option<MemoryLockStatus>, UiError> {
    Ok(state
        .memory_lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .map(|report| MemoryLockStatus {
            locked: report.locked,
            failed: report.failed,
        }))
}

#[tauri::command]
pub async fn panic_now(state: State<'_, AppState>) -> Result<guard::PanicReport, UiError> {
    Ok(guard::panic(&state).await)
}

#[tauri::command]
pub async fn drive_denied_programs(state: State<'_, AppState>) -> Result<Vec<String>, UiError> {
    Ok(state
        .denied_programs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .cloned()
        .collect())
}

#[tauri::command]
pub async fn drive_access_answer(
    state: State<'_, AppState>,
    id: u64,
    allow: bool,
) -> Result<(), UiError> {
    let sender = state
        .access_requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id);
    if let Some(sender) = sender {
        let _ = sender.send(allow);
    }
    Ok(())
}

async fn reauthenticated_set_policy(
    open_vault: &mut crate::state::OpenVault,
    policy: cu_core::VaultPolicy,
    password: Option<&str>,
    key_pin: Option<&str>,
) -> Result<(), cu_core::CoreError> {
    let Some(password) = password else {
        return open_vault.vault.set_policy(policy, None).await;
    };
    if !open_vault.vault.has_two_factor() {
        let credential = cu_core::Credential::Password(password.as_bytes());
        return open_vault.vault.set_policy(policy, Some(&credential)).await;
    }
    let authenticator = cu_core::RealFido2Authenticator::new();
    let secret = open_vault
        .vault
        .two_factor_key_secret(&cu_core::Fido2Auth {
            authenticator: &authenticator,
            pin: key_pin,
        })?;
    let credential = cu_core::Credential::PasswordAndFido2 {
        password: password.as_bytes(),
        fido2_secret: &secret,
    };
    open_vault.vault.set_policy(policy, Some(&credential)).await
}

#[tauri::command]
pub async fn security_policy_get(state: State<'_, AppState>) -> Result<SecurityState, UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    Ok(state_of(&open_vault.vault))
}

#[tauri::command]
pub async fn security_policy_set(
    state: State<'_, AppState>,
    policy: cu_core::VaultPolicy,
    password: Option<String>,
    key_pin: Option<String>,
) -> Result<PolicySetOutcome, UiError> {
    let mut password = password;
    let mut key_pin = key_pin;
    let mut guard = state.vault.lock().await;
    let Some(open_vault) = guard.as_mut() else {
        if let Some(p) = password.as_mut() {
            p.zeroize();
        }
        return Err(UiError::no_vault());
    };

    if open_vault.vault.policy().weakened_by(&policy) && password.is_none() {
        return Ok(PolicySetOutcome::PasswordRequired);
    }
    let result =
        reauthenticated_set_policy(open_vault, policy, password.as_deref(), key_pin.as_deref())
            .await;
    if let Some(p) = key_pin.as_mut() {
        p.zeroize();
    }
    if let Some(p) = password.as_mut() {
        p.zeroize();
    }
    result.map_err(|e| {
        if e.is_decrypt_failure() {
            UiError::coded(&Msg::new("SECURITY_WRONG_PASSWORD"))
        } else {
            UiError::from(e)
        }
    })?;

    state.remember_local_state(open_vault);

    let mut revoked_jobs = Vec::new();
    if open_vault.vault.effective_policy().no_unlock_shortcuts {
        let jobs_path = cu_core::default_jobs_path(&state.data_dir);
        if jobs_path.exists() {
            let location = &open_vault.location;
            revoked_jobs = cu_core::revoke_auto_unlock(&jobs_path, |vault_ref| {
                vault_ref_matches(vault_ref, location)
            })?;
        }
    }
    Ok(PolicySetOutcome::Saved {
        state: state_of(&open_vault.vault),
        revoked_jobs,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatusDto {
    state: &'static str,
    url: Option<String>,
    reason: Option<String>,
}

impl From<cu_core::network::ProxyStatus> for ProxyStatusDto {
    fn from(status: cu_core::network::ProxyStatus) -> Self {
        use cu_core::network::ProxyStatus;
        match status {
            ProxyStatus::Direct => Self {
                state: "direct",
                url: None,
                reason: None,
            },
            ProxyStatus::Active(url) => Self {
                state: "active",
                url: Some(url),
                reason: None,
            },
            ProxyStatus::Broken(reason) => Self {
                state: "broken",
                url: None,
                reason: Some(reason.wire()),
            },
        }
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn network_proxy_get(state: State<'_, AppState>) -> ProxyStatusDto {
    cu_core::network::proxy_status(&state.data_dir).into()
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn network_proxy_set(
    state: State<'_, AppState>,
    url: Option<String>,
) -> Result<ProxyStatusDto, UiError> {
    let url = url.as_deref().map(str::trim).filter(|u| !u.is_empty());
    Ok(cu_core::network::set_proxy(&state.data_dir, url)?.into())
}
