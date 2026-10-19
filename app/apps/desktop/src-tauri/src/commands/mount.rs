// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use tauri::State;

use crate::error::UiError;
use crate::state::AppState;

#[allow(clippy::struct_excessive_bools)]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountStatusDto {
    pub mounted: bool,
    pub mountpoint: Option<String>,
    pub supported: bool,
    pub read_only: bool,
    pub policy_read_only: bool,
    pub pending_files: usize,
}

#[cfg(any(target_os = "linux", windows))]
impl MountStatusDto {
    fn unmounted() -> Self {
        Self {
            mounted: false,
            mountpoint: None,
            supported: true,
            read_only: false,
            policy_read_only: false,
            pending_files: 0,
        }
    }
}

#[cfg(any(target_os = "linux", windows))]
const DEFAULT_CACHE_BUDGET_BYTES: usize = 256 * 1024 * 1024;

#[cfg(target_os = "linux")]
fn home_dir() -> Result<std::path::PathBuf, UiError> {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .map_err(|_| UiError::coded(&Msg::new("MOUNT_NO_HOME")))
}

#[cfg(windows)]
fn home_dir() -> Result<std::path::PathBuf, UiError> {
    std::env::var("USERPROFILE")
        .map(std::path::PathBuf::from)
        .map_err(|_| UiError::coded(&Msg::new("MOUNT_NO_USERPROFILE")))
}

#[cfg(any(target_os = "linux", windows))]
fn vault_name(location: &crate::state::VaultLocation) -> String {
    use crate::state::VaultLocation;
    match location {
        VaultLocation::Local(path) => path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty())
            .unwrap_or("vault")
            .to_string(),
        VaultLocation::Remote(name) => name.clone(),
    }
}

#[cfg(any(target_os = "linux", windows))]
fn default_mountpoint(
    location: &crate::state::VaultLocation,
) -> Result<std::path::PathBuf, UiError> {
    Ok(home_dir()?.join("CryptoUpload").join(vault_name(location)))
}

#[cfg(any(target_os = "linux", windows))]
#[tauri::command]
pub async fn mount_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    read_only: Option<bool>,
    confirm_write: Option<bool>,
) -> Result<MountStatusDto, UiError> {
    use crate::state::MountedVault;
    let confirm_write = confirm_write.unwrap_or(false);

    let mut mount_guard = state.mount.lock().await;
    if mount_guard.is_some() {
        return Err(UiError::coded(&Msg::new("MOUNT_ALREADY")));
    }

    let vault_guard = state.vault.lock().await;
    let open_vault = vault_guard.as_ref().ok_or_else(UiError::no_vault)?;

    let policy_read_only = open_vault.vault.effective_policy().read_only_drive;
    let read_only = read_only.unwrap_or(false) || (policy_read_only && !confirm_write);

    let mountpoint = default_mountpoint(&open_vault.location)?;
    std::fs::create_dir_all(&mountpoint).map_err(|e| {
        UiError::coded(
            &Msg::new("MOUNT_CREATE_DIR")
                .arg(mountpoint.display())
                .arg(&e),
        )
    })?;
    let not_empty = std::fs::read_dir(&mountpoint)
        .map_err(|e| UiError::coded(&Msg::new("MOUNT_READ_DIR").arg(mountpoint.display()).arg(&e)))?
        .next()
        .is_some();
    if not_empty {
        return Err(UiError::coded(
            &Msg::new("MOUNT_NOT_EMPTY").arg(mountpoint.display()),
        ));
    }

    let options = cu_core::MountOptions {
        cache_budget_bytes: DEFAULT_CACHE_BUDGET_BYTES,
        allow_other: false,
        write: if read_only {
            None
        } else {
            Some(cu_core::WriteOptions {
                staging_dir: cu_core::data_dir()
                    .map(|d| cu_core::mount_staging_dir(&d))
                    .map_err(|e| UiError::from_msg(&e))?,
                hooks: None,
            })
        },
    };
    state
        .denied_programs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    let handle = open_vault
        .vault
        .mount_guarded(
            &mountpoint,
            options,
            confirm_write,
            crate::guard::DriveSentry::new(&app),
        )
        .await?;
    drop(vault_guard);

    *mount_guard = Some(MountedVault {
        handle,
        mountpoint: mountpoint.clone(),
        read_only,
    });
    Ok(MountStatusDto {
        mounted: true,
        mountpoint: Some(mountpoint.display().to_string()),
        supported: true,
        read_only,
        policy_read_only,
        pending_files: 0,
    })
}

#[cfg(any(target_os = "linux", windows))]
#[tauri::command]
pub async fn mount_stop(state: State<'_, AppState>) -> Result<(), UiError> {
    let mut mount_guard = state.mount.lock().await;
    let Some(mounted) = mount_guard.take() else {
        return Ok(());
    };
    let flusher = mounted.handle.flusher();
    let _ = flusher.flush().await;
    mounted.handle.unmount()?;
    Ok(())
}

#[cfg(any(target_os = "linux", windows))]
#[tauri::command]
pub async fn mount_status(state: State<'_, AppState>) -> Result<MountStatusDto, UiError> {
    let mount_guard = state.mount.lock().await;
    let policy_read_only = state
        .vault
        .lock()
        .await
        .as_ref()
        .is_some_and(|open| open.vault.effective_policy().read_only_drive);
    Ok(match mount_guard.as_ref() {
        Some(mounted) => MountStatusDto {
            mounted: true,
            mountpoint: Some(mounted.mountpoint.display().to_string()),
            supported: true,
            read_only: mounted.read_only,
            policy_read_only,
            pending_files: mounted.handle.flusher().pending_files(),
        },
        None => MountStatusDto {
            policy_read_only,
            ..MountStatusDto::unmounted()
        },
    })
}

#[cfg(any(target_os = "linux", windows))]
#[tauri::command]
pub async fn mount_open_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), UiError> {
    use tauri_plugin_opener::OpenerExt;
    let mountpoint = {
        let guard = state.mount.lock().await;
        let Some(mounted) = guard.as_ref() else {
            return Err(UiError::coded(&Msg::new("MOUNT_NOT_MOUNTED")));
        };
        mounted.mountpoint.clone()
    };
    app.opener()
        .open_path(mountpoint.display().to_string(), None::<&str>)
        .map_err(|e| {
            UiError::coded(
                &Msg::new("MOUNT_OPEN_FAILED")
                    .arg(mountpoint.display())
                    .arg(&e),
            )
        })
}

#[cfg(not(any(target_os = "linux", windows)))]
#[tauri::command]
pub async fn mount_open_folder() -> Result<(), UiError> {
    Err(UiError::coded(&Msg::new("MOUNT_NOT_AVAILABLE")))
}

#[cfg(not(any(target_os = "linux", windows)))]
#[tauri::command]
pub async fn mount_start(
    _state: State<'_, AppState>,
    _read_only: Option<bool>,
    _confirm_write: Option<bool>,
) -> Result<MountStatusDto, UiError> {
    Err(UiError::coded(&Msg::new("MOUNT_NOT_AVAILABLE")))
}

#[cfg(not(any(target_os = "linux", windows)))]
#[tauri::command]
pub async fn mount_stop(_state: State<'_, AppState>) -> Result<(), UiError> {
    Err(UiError::coded(&Msg::new("MOUNT_NOT_AVAILABLE")))
}

#[cfg(not(any(target_os = "linux", windows)))]
#[tauri::command]
pub async fn mount_status(_state: State<'_, AppState>) -> Result<MountStatusDto, UiError> {
    Ok(MountStatusDto {
        mounted: false,
        mountpoint: None,
        supported: false,
        read_only: false,
        policy_read_only: false,
        pending_files: 0,
    })
}
