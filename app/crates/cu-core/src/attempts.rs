// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::Write as _;
use std::path::{Path, PathBuf};

use cu_jobs::VaultRef;
use serde::{Deserialize, Serialize};

const MAX_RECORDED: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedUnlock {
    pub at: i64,
    pub method: String,
}

fn attempts_path(data_dir: &Path, vault: &VaultRef) -> PathBuf {
    let log_path = crate::activity::ActivityLog::path_for(data_dir, vault);
    let name = log_path
        .file_name()
        .map_or_else(|| "vault.jsonl".into(), std::ffi::OsStr::to_os_string);
    data_dir
        .join("cryptoupload")
        .join("unlock-attempts")
        .join(name)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

pub fn record_failed_unlock(data_dir: &Path, vault: &VaultRef, method: &str) {
    let path = attempts_path(data_dir, vault);
    let already = std::fs::read_to_string(&path).map_or(0, |text| text.lines().count());
    if already >= MAX_RECORDED {
        return;
    }
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let entry = FailedUnlock {
        at: now_unix(),
        method: method.to_string(),
    };
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(&path) {
        let _ = writeln!(file, "{line}");
    }
}

#[must_use]
pub fn take_failed_unlocks(data_dir: &Path, vault: &VaultRef) -> Vec<FailedUnlock> {
    let path = attempts_path(data_dir, vault);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(&path);
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

pub fn clear_all_failed_unlocks(data_dir: &Path) {
    let _ = std::fs::remove_dir_all(data_dir.join("cryptoupload").join("unlock-attempts"));
}
