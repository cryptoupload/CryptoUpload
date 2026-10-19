// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

const FILE_NAME: &str = "ui.json";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Motion {
    #[default]
    Full,
    Reduced,
    Off,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UiSettings {
    pub theme: Theme,
    pub onboarded: bool,
    pub intro_seen: bool,
    pub recent_vaults: Vec<String>,
    pub last_mountpoint: Option<String>,
    pub auto_lock_minutes: u32,
    pub reopen_last_vault: bool,
    pub auto_update_check: bool,
    pub notifications_enabled: bool,
    pub bandwidth_limit_kib: u32,
    pub motion: Motion,
    pub language: Option<String>,
    pub security: AppSecurity,
    pub run_in_background: bool,
    pub start_at_login: bool,
    pub background_hint_seen: bool,
    pub tray_texts: Option<TrayTexts>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrayTexts {
    pub open: String,
    pub quit: String,
    pub notice: String,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSecurity {
    pub hide_from_capture: bool,
    pub lock_memory: bool,
    pub clear_recents_on_exit: bool,
    pub failed_unlock_notice: bool,
    pub panic_shortcut: Option<String>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            onboarded: false,
            intro_seen: false,
            recent_vaults: Vec::new(),
            last_mountpoint: None,
            auto_lock_minutes: 15,
            reopen_last_vault: true,
            auto_update_check: true,
            notifications_enabled: true,
            bandwidth_limit_kib: 0,
            motion: Motion::Full,
            language: None,
            security: AppSecurity::default(),
            run_in_background: true,
            start_at_login: false,
            background_hint_seen: false,
            tray_texts: None,
        }
    }
}

pub const LANGUAGE_SYSTEM: &str = "system";

#[must_use]
pub fn parse_language(value: &str) -> Option<Option<String>> {
    if value == LANGUAGE_SYSTEM {
        return Some(None);
    }
    if (2..=3).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_lowercase()) {
        return Some(Some(value.to_string()));
    }
    None
}

const MAX_RECENT_VAULTS: usize = 5;

pub fn remember_vault(settings: &mut UiSettings, location: &str) {
    settings.recent_vaults.retain(|v| v != location);
    settings.recent_vaults.insert(0, location.to_string());
    settings.recent_vaults.truncate(MAX_RECENT_VAULTS);
}

fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join(FILE_NAME)
}

#[must_use]
pub fn load(data_dir: &Path) -> UiSettings {
    let path = settings_path(data_dir);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

pub fn save(data_dir: &Path, settings: &UiSettings) -> std::io::Result<()> {
    let path = settings_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(settings).unwrap_or_default(),
    )?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
