// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use tauri::AppHandle;
use tauri::State;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroize;

use crate::error::UiError;
use crate::state::{AppState, UnlockedRemote};

#[derive(serde::Serialize)]
#[serde(
    tag = "provider",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RemoteDto {
    S3 {
        name: String,
        endpoint: String,
        bucket: String,
        prefix: String,
        allow_insecure_http: bool,
    },
    Sftp {
        name: String,
        host: String,
        port: u16,
        username: String,
        remote_root: String,
    },
    GDrive {
        name: String,
        folder_id: String,
    },
    Dropbox {
        name: String,
        client_id: String,
    },
    OneDrive {
        name: String,
        client_id: String,
        tenant: String,
        has_client_secret: bool,
        root_kind: OneDriveRootKindDto,
    },
    WebDav {
        name: String,
        base_url: String,
        username: String,
        root_path: String,
        allow_insecure_http: bool,
    },
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum OneDriveRootKindDto {
    AppFolder,
    DriveRoot,
}

impl From<cu_core::OneDriveRootKind> for OneDriveRootKindDto {
    fn from(k: cu_core::OneDriveRootKind) -> Self {
        match k {
            cu_core::OneDriveRootKind::AppFolder => Self::AppFolder,
            cu_core::OneDriveRootKind::DriveRoot => Self::DriveRoot,
        }
    }
}

impl From<OneDriveRootKindDto> for cu_core::OneDriveRootKind {
    fn from(k: OneDriveRootKindDto) -> Self {
        match k {
            OneDriveRootKindDto::AppFolder => Self::AppFolder,
            OneDriveRootKindDto::DriveRoot => Self::DriveRoot,
        }
    }
}

fn to_dto(name: &str, cfg: &cu_core::RemoteConfig) -> RemoteDto {
    match cfg {
        cu_core::RemoteConfig::S3(c) => RemoteDto::S3 {
            name: name.to_string(),
            endpoint: c.endpoint.clone(),
            bucket: c.bucket.clone(),
            prefix: c.prefix.clone(),
            allow_insecure_http: c.allow_insecure_http,
        },
        cu_core::RemoteConfig::Sftp(c) => RemoteDto::Sftp {
            name: name.to_string(),
            host: c.host.clone(),
            port: c.port,
            username: c.username.clone(),
            remote_root: c.remote_root.clone(),
        },
        cu_core::RemoteConfig::GDrive(c) => RemoteDto::GDrive {
            name: name.to_string(),
            folder_id: c.folder_id.clone(),
        },
        cu_core::RemoteConfig::Dropbox(c) => RemoteDto::Dropbox {
            name: name.to_string(),
            client_id: c.client_id.clone(),
        },
        cu_core::RemoteConfig::OneDrive(c) => RemoteDto::OneDrive {
            name: name.to_string(),
            client_id: c.client_id.clone(),
            tenant: c.tenant.clone(),
            has_client_secret: c.client_secret.is_some(),
            root_kind: c.root_kind.into(),
        },
        cu_core::RemoteConfig::WebDav(c) => RemoteDto::WebDav {
            name: name.to_string(),
            base_url: c.base_url.clone(),
            username: c.username.clone(),
            root_path: c.root_path.clone(),
            allow_insecure_http: c.allow_insecure_http,
        },
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemotesStatus {
    Missing,
    Legacy,
    Current,
}

impl From<cu_core::FileFormat> for RemotesStatus {
    fn from(f: cu_core::FileFormat) -> Self {
        match f {
            cu_core::FileFormat::Missing => Self::Missing,
            cu_core::FileFormat::LegacyV1 => Self::Legacy,
            cu_core::FileFormat::Current => Self::Current,
        }
    }
}

#[tauri::command]
pub async fn remotes_status(state: State<'_, AppState>) -> Result<RemotesStatus, UiError> {
    Ok(
        cu_core::SecretsFile::format_version(&state.secrets_file_path())
            .map_err(|e| UiError::from_msg(&e))?
            .into(),
    )
}

#[tauri::command]
pub async fn remotes_migrate_legacy(
    state: State<'_, AppState>,
    old_password: String,
) -> Result<Vec<String>, UiError> {
    let mut old_password = old_password;
    let path = state.secrets_file_path();
    let result = cu_core::migrate_secrets_v1(&path, old_password.as_bytes());
    let migrated = match result {
        Ok(m) => m,
        Err(e) => {
            old_password.zeroize();
            return Err(UiError::from_msg(&e));
        }
    };
    old_password.zeroize();
    migrated.save(&path).map_err(|e| UiError::from_msg(&e))?;
    let mut names: Vec<String> = migrated.names().map(str::to_string).collect();
    names.sort_unstable();
    Ok(names)
}

#[tauri::command]
pub async fn remotes_file_delete(state: State<'_, AppState>) -> Result<(), UiError> {
    state.unlocked_remotes.lock().await.clear();

    if let Err(e) = std::fs::remove_file(state.secrets_file_path())
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(UiError::from_msg(&e));
    }

    let remotes_dir = cu_core::resume::remotes_state_dir(&state.data_dir);
    if let Err(e) = std::fs::remove_dir_all(&remotes_dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        return Err(UiError::from_msg(&e));
    }
    Ok(())
}

#[tauri::command]
pub async fn remote_list_names(state: State<'_, AppState>) -> Result<Vec<String>, UiError> {
    let path = state.secrets_file_path();
    if cu_core::SecretsFile::format_version(&path).map_err(|e| UiError::from_msg(&e))?
        == cu_core::FileFormat::LegacyV1
    {
        return Ok(Vec::new());
    }
    let secrets = cu_core::SecretsFile::load(&path).map_err(|e| UiError::from_msg(&e))?;
    let mut names: Vec<String> = secrets.names().map(str::to_string).collect();
    names.sort_unstable();
    Ok(names)
}

#[tauri::command]
pub async fn remote_unlock(
    state: State<'_, AppState>,
    name: String,
    password: String,
) -> Result<RemoteDto, UiError> {
    let mut password = password;
    let path = state.secrets_file_path();
    let secrets = match cu_core::SecretsFile::load(&path) {
        Ok(s) => s,
        Err(e) => {
            password.zeroize();
            return Err(UiError::from_msg(&e));
        }
    };
    let entry = name.clone();
    let (config, password) = crate::commands::vault::run_kdf(move || {
        match cu_core::remote::unlock_remote(&secrets, &entry, password.as_bytes()) {
            Ok(c) => Ok((c, password)),
            Err(e) => {
                password.zeroize();
                Err(UiError::from_msg(&e))
            }
        }
    })
    .await?;
    let dto = to_dto(&name, &config);
    state
        .unlocked_remotes
        .lock()
        .await
        .insert(name, UnlockedRemote { config, password });
    Ok(dto)
}

#[tauri::command]
pub async fn remote_lock(state: State<'_, AppState>, name: String) -> Result<(), UiError> {
    state.unlocked_remotes.lock().await.remove(&name);
    Ok(())
}

async fn add_and_persist(
    state: &AppState,
    name: String,
    cfg: cu_core::RemoteConfig,
    mut password: String,
) -> Result<(), UiError> {
    let path = state.secrets_file_path();
    crate::commands::vault::run_kdf(move || {
        let result = (|| {
            let mut secrets =
                cu_core::SecretsFile::load(&path).map_err(|e| UiError::from_msg(&e))?;
            cu_core::remote::set_remote(&mut secrets, name, &cfg, password.as_bytes())
                .map_err(|e| UiError::from_msg(&e))?;
            secrets.save(&path).map_err(|e| UiError::from_msg(&e))
        })();
        password.zeroize();
        result
    })
    .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct S3PresetDto {
    id: String,
    label: String,
    has_endpoint: bool,
    regions: Vec<String>,
    path_style: bool,
    note: Option<String>,
}

#[tauri::command]
pub fn s3_presets() -> Vec<S3PresetDto> {
    cu_core::s3_presets()
        .iter()
        .map(|p| S3PresetDto {
            id: p.id.to_string(),
            label: p.label.map_or_else(
                || Msg::new("PRESET_LABEL_CUSTOM").wire(),
                ToString::to_string,
            ),
            has_endpoint: p.endpoint_template.is_some(),
            regions: p.regions.iter().map(ToString::to_string).collect(),
            path_style: p.path_style,
            note: p.note.map(ToString::to_string),
        })
        .collect()
}

#[tauri::command]
pub fn s3_preset_endpoint(preset_id: &str, region: &str) -> Option<String> {
    cu_core::s3_preset_endpoint(preset_id, region)
}

fn parse_object_lock_mode(
    mode: Option<&str>,
) -> Result<Option<cu_core::S3ObjectLockMode>, UiError> {
    match mode {
        None => Ok(None),
        Some(m) if m.eq_ignore_ascii_case("governance") => {
            Ok(Some(cu_core::S3ObjectLockMode::Governance))
        }
        Some(m) if m.eq_ignore_ascii_case("compliance") => {
            Ok(Some(cu_core::S3ObjectLockMode::Compliance))
        }
        Some(other) => Err(UiError::coded(
            &Msg::new("REMOTE_OBJECT_LOCK_MODE").arg(other),
        )),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_add_s3(
    state: State<'_, AppState>,
    name: String,
    endpoint: String,
    region: String,
    bucket: String,
    prefix: String,
    path_style: bool,
    access_key: String,
    secret_key: String,
    password: String,
    object_lock_mode: Option<String>,
    object_lock_days: Option<u16>,
    allow_insecure_http: bool,
) -> Result<(), UiError> {
    let mut secret_key = secret_key;
    let mut password = password;
    let object_lock_mode = parse_object_lock_mode(object_lock_mode.as_deref())?;
    let s3_cfg = cu_core::S3RemoteConfig {
        endpoint,
        region,
        bucket,
        prefix,
        path_style,
        access_key,
        secret_key: secret_key.clone(),
        object_lock_mode,
        object_lock_days,
        allow_insecure_http,
    };
    secret_key.zeroize();
    if let Err(e) = cu_core::remote::verify_s3_exclusive_create(&s3_cfg).await {
        password.zeroize();
        return Err(e.into());
    }
    if let Err(e) = cu_core::remote::verify_s3_object_lock(&s3_cfg).await {
        password.zeroize();
        return Err(e.into());
    }
    let cfg = cu_core::RemoteConfig::S3(s3_cfg);
    add_and_persist(&state, name, cfg, password).await
}

#[tauri::command]
pub async fn remote_allow_insecure_http(
    state: State<'_, AppState>,
    name: String,
) -> Result<RemoteDto, UiError> {
    let (cfg, password) = {
        let unlocked = state.unlocked_remotes.lock().await;
        let Some(entry) = unlocked.get(&name) else {
            return Err(UiError::coded(&Msg::new("REMOTE_NOT_UNLOCKED").arg(&name)));
        };
        let cu_core::RemoteConfig::S3(s3) = &entry.config else {
            return Err(UiError::coded(&Msg::new("REMOTE_NOT_S3")));
        };
        let mut s3 = s3.clone();
        s3.allow_insecure_http = true;
        (cu_core::RemoteConfig::S3(s3), entry.password.clone())
    };
    if let cu_core::RemoteConfig::S3(s3) = &cfg {
        cu_core::remote::check_s3_endpoint_transport(&s3.endpoint, true)?;
    }
    add_and_persist(&state, name.clone(), cfg.clone(), password).await?;
    if let Some(entry) = state.unlocked_remotes.lock().await.get_mut(&name) {
        entry.config = cfg.clone();
    }
    Ok(to_dto(&name, &cfg))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyInfoDto {
    key_type: String,
    fingerprint: String,
}

impl From<cu_core::HostKeyInfo> for HostKeyInfoDto {
    fn from(info: cu_core::HostKeyInfo) -> Self {
        Self {
            key_type: info.key_type,
            fingerprint: info.fingerprint,
        }
    }
}

#[tauri::command]
pub async fn remote_probe_sftp_host_key(
    host: String,
    port: u16,
) -> Result<HostKeyInfoDto, UiError> {
    Ok(cu_core::sftp_probe_host_key(&host, port).await?.into())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_add_sftp(
    state: State<'_, AppState>,
    name: String,
    host: String,
    port: u16,
    username: String,
    auth_method: String,
    auth_password: Option<String>,
    auth_key_path: Option<String>,
    auth_key_passphrase: Option<String>,
    remote_root: String,
    host_key_type: String,
    host_key_fingerprint: String,
    password: String,
) -> Result<(), UiError> {
    let mut auth_password = auth_password;
    let mut auth_key_passphrase = auth_key_passphrase;
    let mut password = password;
    let auth = match auth_method.as_str() {
        "password" => {
            let auth_pw = auth_password
                .clone()
                .ok_or_else(|| UiError::coded(&Msg::new("REMOTE_SSH_PASSWORD_MISSING")));
            let auth_pw = match auth_pw {
                Ok(p) => p,
                Err(e) => {
                    password.zeroize();
                    return Err(e);
                }
            };
            cu_core::SftpAuth::Password { password: auth_pw }
        }
        "key" => {
            let path = match auth_key_path
                .clone()
                .ok_or_else(|| UiError::coded(&Msg::new("REMOTE_SSH_KEY_PATH_MISSING")))
            {
                Ok(p) => p,
                Err(e) => {
                    auth_key_passphrase.zeroize();
                    password.zeroize();
                    return Err(e);
                }
            };
            cu_core::SftpAuth::PrivateKey {
                path,
                passphrase: auth_key_passphrase.clone(),
            }
        }
        other => {
            auth_password.zeroize();
            auth_key_passphrase.zeroize();
            password.zeroize();
            return Err(UiError::coded(
                &Msg::new("REMOTE_SSH_AUTH_UNKNOWN").arg(other),
            ));
        }
    };
    auth_password.zeroize();
    auth_key_passphrase.zeroize();
    let cfg = cu_core::RemoteConfig::Sftp(cu_core::SftpRemoteConfig {
        host,
        port,
        username,
        auth,
        remote_root,
        host_key_type,
        host_key_fingerprint,
    });
    add_and_persist(&state, name, cfg, password).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_add_gdrive(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    client_id: String,
    client_secret: String,
    folder_name: String,
    password: String,
) -> Result<(), UiError> {
    let mut client_secret = client_secret;
    let mut password = password;
    let authorization = match cu_core::begin_gdrive_oauth(&client_id, &client_secret).await {
        Ok(a) => a,
        Err(e) => {
            client_secret.zeroize();
            password.zeroize();
            return Err(e.into());
        }
    };

    if let Err(e) = app.opener().open_url(&authorization.auth_url, None::<&str>) {
        client_secret.zeroize();
        password.zeroize();
        return Err(UiError::coded(
            &Msg::new("REMOTE_BROWSER_OPEN")
                .arg(&e)
                .arg(&authorization.auth_url),
        ));
    }

    let refresh_token = match authorization.wait_for_consent().await {
        Ok(t) => t,
        Err(e) => {
            client_secret.zeroize();
            password.zeroize();
            return Err(e.into());
        }
    };

    let folder_id = match cu_core::gdrive_find_or_create_root_folder(
        &client_id,
        &client_secret,
        &refresh_token,
        &folder_name,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            client_secret.zeroize();
            password.zeroize();
            return Err(e.into());
        }
    };

    let cfg = cu_core::RemoteConfig::GDrive(cu_core::GDriveRemoteConfig {
        client_id,
        client_secret: client_secret.clone(),
        refresh_token,
        folder_id,
    });
    client_secret.zeroize();
    add_and_persist(&state, name, cfg, password).await
}

#[tauri::command]
pub async fn remote_add_dropbox(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    client_id: String,
    password: String,
) -> Result<(), UiError> {
    let mut password = password;
    let authorization = match cu_core::begin_dropbox_oauth(&client_id).await {
        Ok(a) => a,
        Err(e) => {
            password.zeroize();
            return Err(e.into());
        }
    };

    if let Err(e) = app.opener().open_url(&authorization.auth_url, None::<&str>) {
        password.zeroize();
        return Err(UiError::coded(
            &Msg::new("REMOTE_BROWSER_OPEN")
                .arg(&e)
                .arg(&authorization.auth_url),
        ));
    }

    let refresh_token = match authorization.wait_for_consent().await {
        Ok(t) => t,
        Err(e) => {
            password.zeroize();
            return Err(e.into());
        }
    };

    let cfg = cu_core::RemoteConfig::Dropbox(cu_core::DropboxRemoteConfig {
        client_id,
        refresh_token,
    });
    add_and_persist(&state, name, cfg, password).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_add_onedrive(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    client_id: String,
    client_secret: Option<String>,
    tenant: String,
    root_kind: OneDriveRootKindDto,
    password: String,
) -> Result<(), UiError> {
    let mut client_secret = client_secret;
    let mut password = password;
    let root_kind: cu_core::OneDriveRootKind = root_kind.into();
    let authorization = match cu_core::begin_onedrive_oauth(
        &client_id,
        client_secret.as_deref(),
        &tenant,
        root_kind,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            client_secret.zeroize();
            password.zeroize();
            return Err(e.into());
        }
    };

    if let Err(e) = app.opener().open_url(&authorization.auth_url, None::<&str>) {
        client_secret.zeroize();
        password.zeroize();
        return Err(UiError::coded(
            &Msg::new("REMOTE_BROWSER_OPEN")
                .arg(&e)
                .arg(&authorization.auth_url),
        ));
    }

    let refresh_token = match authorization.wait_for_consent().await {
        Ok(t) => t,
        Err(e) => {
            client_secret.zeroize();
            password.zeroize();
            return Err(e.into());
        }
    };

    let cfg = cu_core::RemoteConfig::OneDrive(cu_core::OneDriveRemoteConfig {
        client_id,
        client_secret: client_secret.clone(),
        tenant,
        refresh_token,
        root_kind,
    });
    client_secret.zeroize();
    add_and_persist(&state, name, cfg, password).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_add_webdav(
    state: State<'_, AppState>,
    name: String,
    base_url: String,
    username: String,
    webdav_password: String,
    root_path: String,
    allow_insecure_http: bool,
    password: String,
) -> Result<(), UiError> {
    let mut webdav_password = webdav_password;
    let cfg = cu_core::RemoteConfig::WebDav(cu_core::WebDavRemoteConfig {
        base_url,
        username,
        password: webdav_password.clone(),
        root_path,
        allow_insecure_http,
    });
    webdav_password.zeroize();
    add_and_persist(&state, name, cfg, password).await
}

#[tauri::command]
pub async fn remote_remove(
    state: State<'_, AppState>,
    name: String,
    password: String,
) -> Result<(), UiError> {
    let mut password = password;
    let path = state.secrets_file_path();
    let secrets = match cu_core::SecretsFile::load(&path) {
        Ok(s) => s,
        Err(e) => {
            password.zeroize();
            return Err(UiError::from_msg(&e));
        }
    };
    let entry = name.clone();
    let mut secrets = crate::commands::vault::run_kdf(move || {
        let verified = cu_core::remote::unlock_remote(&secrets, &entry, password.as_bytes());
        password.zeroize();
        verified.map(|_| secrets).map_err(|e| UiError::from_msg(&e))
    })
    .await?;

    secrets.remove(&name);
    secrets.save(&path).map_err(|e| UiError::from_msg(&e))?;
    state.unlocked_remotes.lock().await.remove(&name);
    let _ = cu_core::unlock::forget_remote_password_in_keychain(&name);
    Ok(())
}

#[tauri::command]
pub async fn remote_sweep(
    state: State<'_, AppState>,
    name: String,
    older_than_days: i64,
) -> Result<Vec<String>, UiError> {
    let cfg = {
        let guard = state.unlocked_remotes.lock().await;
        let unlocked = guard
            .get(&name)
            .ok_or_else(|| UiError::coded(&Msg::new("REMOTE_NOT_UNLOCKED").arg(&name)))?;
        unlocked.config.clone()
    };
    let remote_state_db =
        cu_core::resume::remote_state_db_path(&state.data_dir, &cu_core::remote::remote_id(&cfg));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let cutoff_ms = sweep_cutoff_ms(now_ms, older_than_days);
    Ok(cu_core::remote::sweep_remote(&cfg, &remote_state_db, cutoff_ms).await?)
}

fn sweep_cutoff_ms(now_ms: i64, older_than_days: i64) -> i64 {
    now_ms.saturating_sub(older_than_days.max(0).saturating_mul(24 * 60 * 60 * 1000))
}
