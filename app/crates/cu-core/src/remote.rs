// SPDX-License-Identifier: GPL-3.0-or-later

pub use cu_secrets::{
    DropboxRemoteConfig, FileFormat, GDriveRemoteConfig, OneDriveRemoteConfig, OneDriveRootKind,
    RemoteConfig, S3ObjectLockMode, S3RemoteConfig, SecretsError, SecretsFile, SftpAuth,
    SftpRemoteConfig, WebDavRemoteConfig,
};

use std::path::Path;
use std::sync::Arc;

use cu_backend::{NullResumeStore, StorageBackend};
use cu_backend_dropbox::{DropboxBackend, DropboxConfig};
use cu_backend_gdrive::{GDriveBackend, GDriveConfig};
use cu_backend_onedrive::{OneDriveBackend, OneDriveConfig, RootKind as BackendRootKind};
use cu_backend_s3::ObjectLockMode as BackendObjectLockMode;
use cu_backend_s3::{S3Backend, S3Config};
use cu_backend_sftp::{SftpAuth as BackendSftpAuth, SftpBackend, SftpConfig};
use cu_backend_webdav::{WebDavBackend, WebDavConfig};

use crate::error::CoreError;
use crate::resume::SqliteResumeStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyInfo {
    pub key_type: String,
    pub fingerprint: String,
}

pub async fn sftp_probe_host_key(host: &str, port: u16) -> Result<HostKeyInfo, CoreError> {
    let info = cu_backend_sftp::probe_host_key(host, port).await?;
    Ok(HostKeyInfo {
        key_type: info.key_type,
        fingerprint: info.fingerprint,
    })
}

pub struct GDriveAuthorization {
    pub auth_url: String,
    inner: cu_backend_gdrive::oauth::PendingConsent,
}

impl GDriveAuthorization {
    pub async fn wait_for_consent(self) -> Result<String, CoreError> {
        Ok(self.inner.wait_for_consent().await?)
    }
}

pub async fn begin_gdrive_oauth(
    client_id: &str,
    client_secret: &str,
) -> Result<GDriveAuthorization, CoreError> {
    let inner = cu_backend_gdrive::oauth::begin_authorization(client_id, client_secret).await?;
    Ok(GDriveAuthorization {
        auth_url: inner.auth_url.clone(),
        inner,
    })
}

pub async fn gdrive_find_or_create_root_folder(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
    name: &str,
) -> Result<String, CoreError> {
    let cfg = GDriveConfig {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        refresh_token: refresh_token.to_string(),
        folder_id: "root".to_string(),
    };
    let backend = GDriveBackend::new(&cfg, "setup".to_string(), Arc::new(NullResumeStore));
    Ok(backend.find_or_create_root_folder(name).await?)
}

pub struct DropboxAuthorization {
    pub auth_url: String,
    inner: cu_backend_dropbox::oauth::PendingConsent,
}

impl DropboxAuthorization {
    pub async fn wait_for_consent(self) -> Result<String, CoreError> {
        Ok(self.inner.wait_for_consent().await?)
    }
}

pub async fn begin_dropbox_oauth(client_id: &str) -> Result<DropboxAuthorization, CoreError> {
    let inner = cu_backend_dropbox::oauth::begin_authorization(client_id).await?;
    Ok(DropboxAuthorization {
        auth_url: inner.auth_url.clone(),
        inner,
    })
}

fn to_backend_root_kind(root_kind: OneDriveRootKind) -> BackendRootKind {
    match root_kind {
        OneDriveRootKind::AppFolder => BackendRootKind::AppFolder,
        OneDriveRootKind::DriveRoot => BackendRootKind::DriveRoot,
    }
}

pub struct OneDriveAuthorization {
    pub auth_url: String,
    inner: cu_backend_onedrive::oauth::PendingConsent,
}

impl OneDriveAuthorization {
    pub async fn wait_for_consent(self) -> Result<String, CoreError> {
        Ok(self.inner.wait_for_consent().await?)
    }
}

pub async fn begin_onedrive_oauth(
    client_id: &str,
    client_secret: Option<&str>,
    tenant: &str,
    root_kind: OneDriveRootKind,
) -> Result<OneDriveAuthorization, CoreError> {
    let inner = cu_backend_onedrive::oauth::begin_authorization(
        client_id,
        client_secret,
        tenant,
        to_backend_root_kind(root_kind),
    )
    .await?;
    Ok(OneDriveAuthorization {
        auth_url: inner.auth_url.clone(),
        inner,
    })
}

#[must_use]
pub fn remote_id(cfg: &RemoteConfig) -> String {
    use std::fmt::Write as _;
    let mut hasher = cu_crypto::hash::Hasher::new();
    match cfg {
        RemoteConfig::S3(cfg) => {
            hasher.update(cfg.endpoint.as_bytes());
            hasher.update(b"|");
            hasher.update(cfg.bucket.as_bytes());
            hasher.update(b"|");
            hasher.update(cfg.prefix.as_bytes());
        }
        RemoteConfig::Sftp(cfg) => {
            hasher.update(cfg.host.as_bytes());
            hasher.update(b"|");
            hasher.update(cfg.port.to_string().as_bytes());
            hasher.update(b"|");
            hasher.update(cfg.username.as_bytes());
            hasher.update(b"|");
            hasher.update(cfg.remote_root.as_bytes());
        }
        RemoteConfig::GDrive(cfg) => {
            hasher.update(cfg.folder_id.as_bytes());
        }
        RemoteConfig::Dropbox(cfg) => {
            hasher.update(cfg.client_id.as_bytes());
        }
        RemoteConfig::OneDrive(cfg) => {
            hasher.update(cfg.client_id.as_bytes());
            hasher.update(b"|");
            hasher.update(cfg.tenant.as_bytes());
            hasher.update(b"|");
            hasher.update(
                match cfg.root_kind {
                    OneDriveRootKind::AppFolder => "app_folder",
                    OneDriveRootKind::DriveRoot => "drive_root",
                }
                .as_bytes(),
            );
        }
        RemoteConfig::WebDav(cfg) => {
            hasher.update(cfg.base_url.as_bytes());
            hasher.update(b"|");
            hasher.update(cfg.root_path.as_bytes());
        }
    }
    let digest = hasher.finalize();
    digest[..16].iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

pub fn set_remote(
    secrets: &mut SecretsFile,
    name: impl Into<String>,
    cfg: &RemoteConfig,
    password: &[u8],
) -> Result<(), SecretsError> {
    secrets.set(name, cfg, password, cu_crypto::kdf::KdfParams::sensitive())
}

pub fn unlock_remote(
    secrets: &SecretsFile,
    name: &str,
    password: &[u8],
) -> Result<RemoteConfig, SecretsError> {
    secrets.unlock(name, password)
}

pub fn migrate_secrets_v1(path: &Path, old_password: &[u8]) -> Result<SecretsFile, SecretsError> {
    SecretsFile::migrate_v1(path, old_password, cu_crypto::kdf::KdfParams::sensitive())
}

pub type RefreshTokenSink = Arc<dyn Fn(String) + Send + Sync>;

#[must_use]
pub fn with_rotated_refresh_token(cfg: &RemoteConfig, new_token: String) -> Option<RemoteConfig> {
    let mut cfg = cfg.clone();
    match &mut cfg {
        RemoteConfig::GDrive(c) => c.refresh_token = new_token,
        RemoteConfig::Dropbox(c) => c.refresh_token = new_token,
        RemoteConfig::OneDrive(c) => c.refresh_token = new_token,
        RemoteConfig::S3(_) | RemoteConfig::Sftp(_) | RemoteConfig::WebDav(_) => return None,
    }
    Some(cfg)
}

pub fn persist_rotated_refresh_token(
    secrets_path: &Path,
    name: &str,
    password: &[u8],
    new_token: String,
) -> Result<bool, SecretsError> {
    persist_rotated_refresh_token_with(
        secrets_path,
        name,
        password,
        new_token,
        cu_crypto::kdf::KdfParams::sensitive(),
    )
}

fn persist_rotated_refresh_token_with(
    secrets_path: &Path,
    name: &str,
    password: &[u8],
    new_token: String,
    params: cu_crypto::kdf::KdfParams,
) -> Result<bool, SecretsError> {
    let mut secrets = SecretsFile::load(secrets_path)?;
    let current = secrets.unlock(name, password)?;
    let Some(updated) = with_rotated_refresh_token(&current, new_token) else {
        return Ok(false);
    };
    secrets.set(name, &updated, password, params)?;
    secrets.save(secrets_path)?;
    Ok(true)
}

#[must_use]
pub fn persisting_refresh_sink(
    secrets_path: std::path::PathBuf,
    name: String,
    password: Vec<u8>,
) -> RefreshTokenSink {
    persisting_refresh_sink_with(
        secrets_path,
        name,
        password,
        cu_crypto::kdf::KdfParams::sensitive(),
    )
}

fn persisting_refresh_sink_with(
    secrets_path: std::path::PathBuf,
    name: String,
    password: Vec<u8>,
    params: cu_crypto::kdf::KdfParams,
) -> RefreshTokenSink {
    struct Shared {
        secrets_path: std::path::PathBuf,
        name: String,
        password: zeroize::Zeroizing<Vec<u8>>,
        params: cu_crypto::kdf::KdfParams,
        latest: std::sync::Mutex<u64>,
        issued: std::sync::atomic::AtomicU64,
    }
    let shared = Arc::new(Shared {
        secrets_path,
        name,
        password: zeroize::Zeroizing::new(password),
        params,
        latest: std::sync::Mutex::new(0),
        issued: std::sync::atomic::AtomicU64::new(0),
    });
    Arc::new(move |new_token: String| {
        let shared = Arc::clone(&shared);
        let seq = shared
            .issued
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let handle = std::thread::spawn(move || {
            let mut latest = shared
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if seq < *latest {
                return;
            }
            *latest = seq;
            if let Err(e) = persist_rotated_refresh_token_with(
                &shared.secrets_path,
                &shared.name,
                &shared.password,
                new_token,
                shared.params,
            ) {
                eprintln!(
                    "remote \"{}\": refresh token not persisted: {e}",
                    shared.name
                );
            }
        });
        PENDING_REFRESH_SAVES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(handle);
    })
}

static PENDING_REFRESH_SAVES: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> =
    std::sync::Mutex::new(Vec::new());

pub fn wait_for_pending_refresh_token_saves() {
    let pending = std::mem::take(
        &mut *PENDING_REFRESH_SAVES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    for handle in pending {
        let _ = handle.join();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Preset {
    pub id: &'static str,
    pub label: Option<&'static str>,
    pub endpoint_template: Option<&'static str>,
    pub regions: &'static [&'static str],
    pub path_style: bool,
    pub note: Option<&'static str>,
}

#[must_use]
pub fn s3_presets() -> &'static [S3Preset] {
    &[
        S3Preset {
            id: "custom",
            label: None,
            endpoint_template: None,
            regions: &[],
            path_style: true,
            note: None,
        },
        S3Preset {
            id: "aws",
            label: Some("AWS S3"),
            endpoint_template: Some("https://s3.{region}.amazonaws.com"),
            regions: &["eu-central-1", "eu-west-1", "us-east-1", "us-west-2"],
            path_style: false,
            note: None,
        },
        S3Preset {
            id: "minio",
            label: Some("MinIO"),
            endpoint_template: Some("http://127.0.0.1:9000"),
            regions: &["us-east-1"],
            path_style: true,
            note: Some("PRESET_NOTE_MINIO"),
        },
        S3Preset {
            id: "b2",
            label: Some("Backblaze B2"),
            endpoint_template: Some("https://s3.{region}.backblazeb2.com"),
            regions: &[
                "us-west-004",
                "us-west-002",
                "us-east-005",
                "eu-central-003",
            ],
            path_style: false,
            note: Some("PRESET_NOTE_B2"),
        },
        S3Preset {
            id: "wasabi",
            label: Some("Wasabi"),
            endpoint_template: Some("https://s3.{region}.wasabisys.com"),
            regions: &[
                "us-east-1",
                "us-east-2",
                "us-central-1",
                "us-west-1",
                "eu-central-1",
                "eu-central-2",
                "eu-west-1",
                "eu-west-2",
                "ap-northeast-1",
                "ap-southeast-1",
                "ca-central-1",
            ],
            path_style: false,
            note: Some("PRESET_NOTE_WASABI"),
        },
    ]
}

#[must_use]
pub fn s3_preset_endpoint(preset_id: &str, region: &str) -> Option<String> {
    s3_presets()
        .iter()
        .find(|p| p.id == preset_id)
        .and_then(|p| p.endpoint_template)
        .map(|tpl| tpl.replace("{region}", region))
}

fn s3_config(cfg: &S3RemoteConfig) -> Result<S3Config, CoreError> {
    check_s3_endpoint_transport(&cfg.endpoint, cfg.allow_insecure_http)?;
    Ok(S3Config {
        endpoint: Some(cfg.endpoint.clone()),
        region: cfg.region.clone(),
        bucket: cfg.bucket.clone(),
        prefix: cfg.prefix.clone(),
        path_style: cfg.path_style,
        access_key: cfg.access_key.clone(),
        secret_key: cfg.secret_key.clone(),
        object_lock_mode: cfg.object_lock_mode.map(|m| match m {
            S3ObjectLockMode::Governance => BackendObjectLockMode::Governance,
            S3ObjectLockMode::Compliance => BackendObjectLockMode::Compliance,
        }),
        object_lock_days: cfg.object_lock_days,
    })
}

pub async fn verify_s3_object_lock(cfg: &S3RemoteConfig) -> Result<(), CoreError> {
    if cfg.object_lock_mode.is_none() {
        return Ok(());
    }
    S3Backend::check_object_lock_enabled(&s3_config(cfg)?).await?;
    Ok(())
}

fn endpoint_host(endpoint: &str) -> &str {
    let rest = endpoint.split_once("://").map_or(endpoint, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    }
}

fn is_loopback_endpoint(endpoint: &str) -> bool {
    let host = endpoint_host(endpoint);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

pub fn check_s3_endpoint_transport(
    endpoint: &str,
    allow_insecure_http: bool,
) -> Result<(), CoreError> {
    let Some((scheme, _)) = endpoint.trim().split_once("://") else {
        return Ok(());
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return Ok(());
    }
    let msg = if !is_private_or_loopback_endpoint(endpoint.trim()) {
        cu_backend::Msg::new("S3_HTTP_PUBLIC_HOST")
    } else if !allow_insecure_http {
        cu_backend::Msg::new("S3_HTTP_NOT_CONFIRMED")
    } else {
        return Ok(());
    };
    Err(CoreError::Backend(cu_backend::BackendError::Coded(msg)))
}

fn is_private_or_loopback_endpoint(endpoint: &str) -> bool {
    if is_loopback_endpoint(endpoint) {
        return true;
    }
    match endpoint_host(endpoint).parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => {
            let first = ip.segments()[0];
            first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
        }
        Err(_) => false,
    }
}

pub async fn verify_s3_exclusive_create(cfg: &S3RemoteConfig) -> Result<(), CoreError> {
    let s3 = s3_config(cfg)?;
    cu_backend_s3::check_proxy_supported()?;
    let backend = S3Backend::new(
        &s3,
        remote_id(&RemoteConfig::S3(cfg.clone())),
        Arc::new(cu_backend::NullResumeStore),
    );
    if cu_backend::probe_exclusive_create(&backend).await? {
        Ok(())
    } else {
        Err(CoreError::Backend(cu_backend::BackendError::Coded(
            cu_backend::Msg::new("S3_NO_EXCLUSIVE_WRITE"),
        )))
    }
}

fn sftp_config(cfg: &SftpRemoteConfig) -> SftpConfig {
    SftpConfig {
        host: cfg.host.clone(),
        port: cfg.port,
        username: cfg.username.clone(),
        auth: match &cfg.auth {
            SftpAuth::Password { password } => BackendSftpAuth::Password(password.clone()),
            SftpAuth::PrivateKey { path, passphrase } => BackendSftpAuth::PrivateKey {
                path: path.clone(),
                passphrase: passphrase.clone(),
            },
        },
        remote_root: cfg.remote_root.clone(),
        expected_host_key_fingerprint: cfg.host_key_fingerprint.clone(),
    }
}

fn gdrive_config(cfg: &GDriveRemoteConfig) -> GDriveConfig {
    GDriveConfig {
        client_id: cfg.client_id.clone(),
        client_secret: cfg.client_secret.clone(),
        refresh_token: cfg.refresh_token.clone(),
        folder_id: cfg.folder_id.clone(),
    }
}

fn dropbox_config(cfg: &DropboxRemoteConfig) -> DropboxConfig {
    DropboxConfig {
        client_id: cfg.client_id.clone(),
        refresh_token: cfg.refresh_token.clone(),
    }
}

fn webdav_config(cfg: &WebDavRemoteConfig) -> WebDavConfig {
    WebDavConfig {
        base_url: cfg.base_url.clone(),
        username: cfg.username.clone(),
        password: cfg.password.clone(),
        root_path: cfg.root_path.clone(),
        allow_insecure_http: cfg.allow_insecure_http,
    }
}

fn onedrive_config(cfg: &OneDriveRemoteConfig) -> OneDriveConfig {
    OneDriveConfig {
        client_id: cfg.client_id.clone(),
        client_secret: cfg.client_secret.clone(),
        tenant: cfg.tenant.clone(),
        refresh_token: cfg.refresh_token.clone(),
        root_kind: to_backend_root_kind(cfg.root_kind),
    }
}

fn open_resume_store(path: &Path) -> Result<SqliteResumeStore, CoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    SqliteResumeStore::open(path)
}

pub(crate) fn wrap_bandwidth(
    backend: Box<dyn StorageBackend>,
    bandwidth_limit_kib: Option<u32>,
) -> Box<dyn StorageBackend> {
    match bandwidth_limit_kib {
        Some(kib) if kib > 0 => {
            let limiter = Arc::new(cu_backend::throttle::RateLimiter::new(
                u64::from(kib) * 1024,
            ));
            Box::new(cu_backend::throttle::ThrottleBackend::new(backend, limiter))
        }
        _ => backend,
    }
}

pub(crate) async fn build_backend_with_refresh_sink(
    cfg: &RemoteConfig,
    remote_state_db: &Path,
    bandwidth_limit_kib: Option<u32>,
    refresh_sink: Option<RefreshTokenSink>,
) -> Result<Box<dyn StorageBackend>, CoreError> {
    let id = remote_id(cfg);
    let backend: Box<dyn StorageBackend> = match cfg {
        RemoteConfig::S3(s3_cfg) => {
            let s3 = s3_config(s3_cfg)?;
            cu_backend_s3::check_proxy_supported()?;
            let resume = Arc::new(open_resume_store(remote_state_db)?);
            Box::new(S3Backend::new(&s3, id, resume))
        }
        RemoteConfig::Sftp(sftp_cfg) => {
            Box::new(SftpBackend::connect(&sftp_config(sftp_cfg)).await?)
        }
        RemoteConfig::GDrive(gdrive_cfg) => {
            let resume = Arc::new(open_resume_store(remote_state_db)?);
            let backend = GDriveBackend::new(&gdrive_config(gdrive_cfg), id, resume);
            if let Some(sink) = refresh_sink {
                backend.set_refresh_token_sink(sink);
            }
            Box::new(backend)
        }
        RemoteConfig::Dropbox(dropbox_cfg) => {
            let resume = Arc::new(open_resume_store(remote_state_db)?);
            Box::new(DropboxBackend::new(
                &dropbox_config(dropbox_cfg),
                id,
                resume,
            ))
        }
        RemoteConfig::OneDrive(onedrive_cfg) => {
            let resume = Arc::new(open_resume_store(remote_state_db)?);
            let backend = OneDriveBackend::new(&onedrive_config(onedrive_cfg), id, resume);
            if let Some(sink) = refresh_sink {
                backend.set_refresh_token_sink(sink);
            }
            Box::new(backend)
        }
        RemoteConfig::WebDav(webdav_cfg) => {
            let resume = Arc::new(open_resume_store(remote_state_db)?);
            Box::new(WebDavBackend::new(&webdav_config(webdav_cfg), id, resume)?)
        }
    };
    let backend = wrap_bandwidth(backend, bandwidth_limit_kib);
    Ok(backend)
}

pub async fn sweep_remote(
    cfg: &RemoteConfig,
    remote_state_db: &Path,
    cutoff_ms: i64,
) -> Result<Vec<String>, CoreError> {
    match cfg {
        RemoteConfig::S3(s3_cfg) => {
            let id = remote_id(cfg);
            let s3 = s3_config(s3_cfg)?;
            cu_backend_s3::check_proxy_supported()?;
            let resume = Arc::new(open_resume_store(remote_state_db)?);
            let backend = S3Backend::new(&s3, id, resume);
            Ok(backend.sweep_orphans(cutoff_ms).await?)
        }
        RemoteConfig::WebDav(webdav_cfg) => {
            let id = remote_id(cfg);
            let resume = Arc::new(open_resume_store(remote_state_db)?);
            let backend = WebDavBackend::new(&webdav_config(webdav_cfg), id, resume)?;
            Ok(backend.sweep_orphans(cutoff_ms).await?)
        }
        _ => Ok(Vec::new()),
    }
}
