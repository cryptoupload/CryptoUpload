// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use tauri::State;

use crate::error::UiError;
use crate::settings::{Motion, Theme, UiSettings};
use crate::state::AppState;

#[allow(clippy::struct_excessive_bools)]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSettingsDto {
    theme: Theme,
    onboarded: bool,
    intro_seen: bool,
    recent_vaults: Vec<String>,
    last_mountpoint: Option<String>,
    auto_lock_minutes: u32,
    reopen_last_vault: bool,
    auto_update_check: bool,
    notifications_enabled: bool,
    bandwidth_limit_kib: u32,
    motion: Motion,
    language: Option<String>,
    run_in_background: bool,
    start_at_login: bool,
}

impl From<&UiSettings> for UiSettingsDto {
    fn from(settings: &UiSettings) -> Self {
        Self {
            theme: settings.theme,
            onboarded: settings.onboarded,
            intro_seen: settings.intro_seen,
            recent_vaults: settings.recent_vaults.clone(),
            last_mountpoint: settings.last_mountpoint.clone(),
            auto_lock_minutes: settings.auto_lock_minutes,
            reopen_last_vault: settings.reopen_last_vault,
            auto_update_check: settings.auto_update_check,
            notifications_enabled: settings.notifications_enabled,
            bandwidth_limit_kib: settings.bandwidth_limit_kib,
            motion: settings.motion,
            language: settings.language.clone(),
            run_in_background: settings.run_in_background,
            start_at_login: settings.start_at_login,
        }
    }
}

#[tauri::command]
pub async fn ui_settings_get(state: State<'_, AppState>) -> Result<UiSettingsDto, UiError> {
    let settings = state.settings.lock().await;
    Ok(UiSettingsDto::from(&*settings))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ui_settings_set(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    theme: Option<Theme>,
    onboarded: Option<bool>,
    intro_seen: Option<bool>,
    auto_lock_minutes: Option<u32>,
    reopen_last_vault: Option<bool>,
    auto_update_check: Option<bool>,
    notifications_enabled: Option<bool>,
    bandwidth_limit_kib: Option<u32>,
    motion: Option<Motion>,
    language: Option<String>,
    run_in_background: Option<bool>,
    start_at_login: Option<bool>,
) -> Result<UiSettingsDto, UiError> {
    let mut settings = state.settings.lock().await;
    if let Some(theme) = theme {
        settings.theme = theme;
    }
    if let Some(onboarded) = onboarded {
        settings.onboarded = onboarded;
    }
    if let Some(intro_seen) = intro_seen {
        settings.intro_seen = intro_seen;
    }
    if let Some(auto_lock_minutes) = auto_lock_minutes {
        settings.auto_lock_minutes = auto_lock_minutes;
    }
    if let Some(reopen_last_vault) = reopen_last_vault {
        settings.reopen_last_vault = reopen_last_vault;
    }
    if let Some(auto_update_check) = auto_update_check {
        settings.auto_update_check = auto_update_check;
    }
    if let Some(notifications_enabled) = notifications_enabled {
        settings.notifications_enabled = notifications_enabled;
    }
    if let Some(bandwidth_limit_kib) = bandwidth_limit_kib {
        settings.bandwidth_limit_kib = bandwidth_limit_kib;
    }
    if let Some(motion) = motion {
        settings.motion = motion;
    }
    if let Some(language) = language
        && let Some(parsed) = crate::settings::parse_language(&language)
    {
        settings.language = parsed;
    }
    if let Some(run_in_background) = run_in_background {
        settings.run_in_background = run_in_background;
    }
    if let Some(start_at_login) = start_at_login {
        if crate::background::set_start_at_login(&app, start_at_login) {
            settings.start_at_login = start_at_login;
        } else {
            return Err(UiError::coded(&Msg::new("START_AT_LOGIN_FAILED")));
        }
    }
    if let Err(e) = crate::settings::save(&state.data_dir, &settings) {
        eprintln!("cannot save ui.json: {e}");
    }
    Ok(UiSettingsDto::from(&*settings))
}

const TRANSLATION_FILE_MAX_BYTES: usize = 2 * 1024 * 1024;

#[tauri::command]
pub async fn translation_export(path: String, content: String) -> Result<(), UiError> {
    if content.len() > TRANSLATION_FILE_MAX_BYTES {
        return Err(UiError::coded(&Msg::new("TRANSLATION_FILE_TOO_LARGE")));
    }
    if serde_json::from_str::<serde_json::Value>(&content).is_err() {
        return Err(UiError::coded(&Msg::new("TRANSLATION_FILE_INVALID")));
    }
    let path = std::path::PathBuf::from(path);
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return Err(UiError::coded(&Msg::new("TRANSLATION_FILE_INVALID")));
    }
    std::fs::write(&path, content).map_err(|e| UiError::coded(&Msg::new("IO").arg(&e)))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BugReportInfo {
    version: String,
    os: &'static str,
    arch: &'static str,
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn bug_report_info(app: tauri::AppHandle) -> BugReportInfo {
    BugReportInfo {
        version: app.package_info().version.to_string(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn app_restart(app: tauri::AppHandle) {
    app.request_restart();
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn set_outside_texts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    consent_reason: String,
    oauth_granted: String,
    oauth_denied: String,
    tray_open: String,
    tray_quit: String,
    background_notice: String,
    sleep_reason: Option<String>,
) -> Result<(), UiError> {
    cu_core::set_consent_reason(consent_reason);
    if let Some(text) = sleep_reason.filter(|t| !t.trim().is_empty()) {
        cu_core::sentinel::set_sleep_reason(text);
    }
    cu_core::set_oauth_pages(oauth_granted, oauth_denied);
    let texts = crate::settings::TrayTexts {
        open: tray_open,
        quit: tray_quit,
        notice: background_notice,
    };
    let mut settings = state.settings.lock().await;
    if settings.tray_texts.as_ref() != Some(&texts) {
        settings.tray_texts = Some(texts.clone());
        let _ = crate::settings::save(&state.data_dir, &settings);
    }
    drop(settings);
    crate::background::show_tray(&app, &texts);
    Ok(())
}
