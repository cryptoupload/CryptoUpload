// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::{Path, PathBuf};

use crate::error::CoreError;

pub fn data_dir() -> Result<PathBuf, CoreError> {
    if cfg!(target_os = "macos") {
        let home = std::env::var("HOME")
            .map_err(|_| CoreError::DataDirUnavailable(Msg::new("DATA_DIR_NO_HOME")))?;
        return Ok(PathBuf::from(home).join("Library/Application Support"));
    }
    if cfg!(target_os = "windows") {
        let appdata = std::env::var("APPDATA")
            .map_err(|_| CoreError::DataDirUnavailable(Msg::new("DATA_DIR_NO_APPDATA")))?;
        return Ok(PathBuf::from(appdata));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg));
    }
    let home = std::env::var("HOME")
        .map_err(|_| CoreError::DataDirUnavailable(Msg::new("DATA_DIR_NO_HOME")))?;
    Ok(PathBuf::from(home).join(".local/share"))
}

#[must_use]
pub fn secrets_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join("remotes.secrets")
}

#[must_use]
pub fn mount_staging_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join("mount-staging")
}
