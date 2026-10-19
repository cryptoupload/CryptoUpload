// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use cu_crypto::aead::{open_with_aad, seal_with_aad};
use cu_crypto::kdf::{KdfParams, derive_kek};
use serde::{Deserialize, Serialize};

use crate::error::SecretsError;

const MAGIC: &[u8; 5] = b"cusec";
const LEGACY_VERSION: u8 = 1;
const VERSION: u8 = 2;
const LEGACY_HEADER_LEN: usize = 5 + 1 + 4 + 4 + 4 + crate::SALT_LEN;
const ENTRY_HEADER_LEN: usize = 4 + 4 + 4 + crate::SALT_LEN;
const PAD_BLOCK: usize = 256;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum RemoteConfig {
    S3(S3RemoteConfig),
    Sftp(SftpRemoteConfig),
    GDrive(GDriveRemoteConfig),
    Dropbox(DropboxRemoteConfig),
    OneDrive(OneDriveRemoteConfig),
    WebDav(WebDavRemoteConfig),
}

impl fmt::Debug for RemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::S3(cfg) => f.debug_tuple("RemoteConfig::S3").field(cfg).finish(),
            Self::Sftp(cfg) => f.debug_tuple("RemoteConfig::Sftp").field(cfg).finish(),
            Self::GDrive(cfg) => f.debug_tuple("RemoteConfig::GDrive").field(cfg).finish(),
            Self::Dropbox(cfg) => f.debug_tuple("RemoteConfig::Dropbox").field(cfg).finish(),
            Self::OneDrive(cfg) => f.debug_tuple("RemoteConfig::OneDrive").field(cfg).finish(),
            Self::WebDav(cfg) => f.debug_tuple("RemoteConfig::WebDav").field(cfg).finish(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct S3RemoteConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
    pub access_key: String,
    pub secret_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_lock_mode: Option<S3ObjectLockMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_lock_days: Option<u16>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_insecure_http: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum S3ObjectLockMode {
    Governance,
    Compliance,
}

impl fmt::Debug for S3RemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3RemoteConfig")
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("path_style", &self.path_style)
            .field("access_key", &"<redacted>")
            .field("secret_key", &"<redacted>")
            .field("object_lock_mode", &self.object_lock_mode)
            .field("object_lock_days", &self.object_lock_days)
            .field("allow_insecure_http", &self.allow_insecure_http)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum SftpAuth {
    Password {
        password: String,
    },
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SftpRemoteConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SftpAuth,
    pub remote_root: String,
    pub host_key_type: String,
    pub host_key_fingerprint: String,
}

impl fmt::Debug for SftpAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password { .. } => f
                .debug_struct("Password")
                .field("password", &"<redacted>")
                .finish(),
            Self::PrivateKey { path, .. } => f
                .debug_struct("PrivateKey")
                .field("path", path)
                .field("passphrase", &"<redacted>")
                .finish(),
        }
    }
}

impl fmt::Debug for SftpRemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SftpRemoteConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("auth", &self.auth)
            .field("remote_root", &self.remote_root)
            .field("host_key_type", &self.host_key_type)
            .field("host_key_fingerprint", &self.host_key_fingerprint)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GDriveRemoteConfig {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub folder_id: String,
}

impl fmt::Debug for GDriveRemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GDriveRemoteConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("folder_id", &self.folder_id)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropboxRemoteConfig {
    pub client_id: String,
    pub refresh_token: String,
}

impl fmt::Debug for DropboxRemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropboxRemoteConfig")
            .field("client_id", &self.client_id)
            .field("refresh_token", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OneDriveRootKind {
    AppFolder,
    DriveRoot,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OneDriveRemoteConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub tenant: String,
    pub refresh_token: String,
    pub root_kind: OneDriveRootKind,
}

impl fmt::Debug for OneDriveRemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OneDriveRemoteConfig")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("tenant", &self.tenant)
            .field("refresh_token", &"<redacted>")
            .field("root_kind", &self.root_kind)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebDavRemoteConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub root_path: String,
    pub allow_insecure_http: bool,
}

impl fmt::Debug for WebDavRemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebDavRemoteConfig")
            .field("base_url", &self.base_url)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("root_path", &self.root_path)
            .field("allow_insecure_http", &self.allow_insecure_http)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct EntryHeader {
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
    salt: [u8; crate::SALT_LEN],
}

impl EntryHeader {
    fn params(&self) -> Result<KdfParams, cu_crypto::CryptoError> {
        KdfParams::from_untrusted(self.m_cost_kib, self.t_cost, self.p_cost)
    }

    fn to_bytes(&self) -> [u8; ENTRY_HEADER_LEN] {
        let mut out = [0u8; ENTRY_HEADER_LEN];
        out[0..4].copy_from_slice(&self.m_cost_kib.to_le_bytes());
        out[4..8].copy_from_slice(&self.t_cost.to_le_bytes());
        out[8..12].copy_from_slice(&self.p_cost.to_le_bytes());
        out[12..12 + crate::SALT_LEN].copy_from_slice(&self.salt);
        out
    }
}

#[derive(Clone)]
struct StoredEntry {
    header: EntryHeader,
    sealed: Vec<u8>,
}

#[derive(Default)]
pub struct SecretsFile {
    entries: HashMap<String, StoredEntry>,
}

impl fmt::Debug for SecretsFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names: Vec<&str> = self.entries.keys().map(String::as_str).collect();
        names.sort_unstable();
        f.debug_struct("SecretsFile")
            .field("names", &names)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Missing,
    LegacyV1,
    Current,
}

impl SecretsFile {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[allow(clippy::indexing_slicing)]
    pub fn format_version(path: &Path) -> Result<FileFormat, SecretsError> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileFormat::Missing),
            Err(e) => return Err(e.into()),
        };
        if bytes.len() < 6 {
            return Err(SecretsError::Malformed("file too short"));
        }
        if &bytes[0..5] != MAGIC {
            return Err(SecretsError::Malformed("magic bytes missing"));
        }
        match bytes[5] {
            LEGACY_VERSION => Ok(FileFormat::LegacyV1),
            VERSION => Ok(FileFormat::Current),
            v => Err(SecretsError::UnsupportedVersion(v)),
        }
    }

    #[allow(clippy::indexing_slicing)]
    pub fn load(path: &Path) -> Result<Self, SecretsError> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::empty()),
            Err(e) => return Err(e.into()),
        };
        if bytes.len() < 6 {
            return Err(SecretsError::Malformed("file too short"));
        }
        if &bytes[0..5] != MAGIC {
            return Err(SecretsError::Malformed("magic bytes missing"));
        }
        match bytes[5] {
            VERSION => Ok(Self {
                entries: decode_v2(&bytes)?,
            }),
            LEGACY_VERSION => Err(SecretsError::LegacyFormat),
            v => Err(SecretsError::UnsupportedVersion(v)),
        }
    }

    pub fn migrate_v1(
        path: &Path,
        old_password: &[u8],
        params: KdfParams,
    ) -> Result<Self, SecretsError> {
        let bytes = std::fs::read(path)?;
        let remotes = decode_legacy_v1(&bytes, old_password)?;
        let mut file = Self::empty();
        for (name, cfg) in &remotes {
            file.set(name.clone(), cfg, old_password, params)?;
        }
        Ok(file)
    }

    pub fn save(&self, path: &Path) -> Result<(), SecretsError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let out = encode_v2(&self.entries);
        let tmp = tmp_path(path);
        write_private(&tmp, &out)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn unlock(&self, name: &str, password: &[u8]) -> Result<RemoteConfig, SecretsError> {
        let entry = self.entries.get(name).ok_or(SecretsError::NoSuchEntry)?;
        let aad = entry_aad(name, &entry.header);
        let kek = derive_kek(password, &entry.header.salt, entry.header.params()?)?;
        let padded = zeroize::Zeroizing::new(open_with_aad(&kek, &aad, &entry.sealed)?);
        let plaintext = zeroize::Zeroizing::new(unpad(&padded)?);
        Ok(serde_json::from_slice(&plaintext)?)
    }

    pub fn set(
        &mut self,
        name: impl Into<String>,
        cfg: &RemoteConfig,
        password: &[u8],
        params: KdfParams,
    ) -> Result<(), SecretsError> {
        let name = name.into();
        if name.len() > usize::from(u16::MAX) {
            return Err(SecretsError::Malformed(
                "remote name too long (max 65535 UTF-8 bytes)",
            ));
        }

        let mut salt = [0u8; crate::SALT_LEN];
        cu_crypto::random::fill_random(&mut salt)?;
        let header = EntryHeader {
            m_cost_kib: params.m_cost_kib(),
            t_cost: params.t_cost(),
            p_cost: params.p_cost(),
            salt,
        };
        let aad = entry_aad(&name, &header);
        let kek = derive_kek(password, &salt, params)?;
        let plaintext = zeroize::Zeroizing::new(serde_json::to_vec(cfg)?);
        let padded = zeroize::Zeroizing::new(pad(&plaintext));
        let sealed = seal_with_aad(&kek, &aad, &padded)?;

        self.entries.insert(name, StoredEntry { header, sealed });
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

fn entry_aad(name: &str, header: &EntryHeader) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let mut aad = Vec::with_capacity(5 + 1 + 2 + name_bytes.len() + ENTRY_HEADER_LEN);
    aad.extend_from_slice(MAGIC);
    aad.push(VERSION);
    #[allow(clippy::cast_possible_truncation)]
    aad.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    aad.extend_from_slice(name_bytes);
    aad.extend_from_slice(&header.to_bytes());
    aad
}

fn pad(plaintext: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + plaintext.len());
    let len = u32::try_from(plaintext.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(plaintext);
    let padded_len = out.len().div_ceil(PAD_BLOCK).max(1) * PAD_BLOCK;
    out.resize(padded_len, 0);
    out
}

#[allow(clippy::indexing_slicing, clippy::expect_used)]
fn unpad(padded: &[u8]) -> Result<Vec<u8>, SecretsError> {
    if padded.len() < 4 {
        return Err(SecretsError::Malformed("padded entry too short"));
    }
    let len = u32::from_le_bytes(padded[0..4].try_into().expect("4-byte slice")) as usize;
    if 4 + len > padded.len() {
        return Err(SecretsError::Malformed(
            "declared length beyond the padded entry",
        ));
    }
    Ok(padded[4..4 + len].to_vec())
}

#[allow(clippy::indexing_slicing, clippy::expect_used)]
fn decode_v2(bytes: &[u8]) -> Result<HashMap<String, StoredEntry>, SecretsError> {
    if bytes.len() < 10 {
        return Err(SecretsError::Malformed("file header truncated"));
    }
    let count = u32::from_le_bytes(bytes[6..10].try_into().expect("4-byte slice"));

    let mut entries = HashMap::new();
    let mut cursor = 10usize;
    for _ in 0..count {
        if cursor + 2 > bytes.len() {
            return Err(SecretsError::Malformed("entry header truncated"));
        }
        let name_len =
            u16::from_le_bytes(bytes[cursor..cursor + 2].try_into().expect("2-byte slice"))
                as usize;
        cursor += 2;

        if cursor + name_len > bytes.len() {
            return Err(SecretsError::Malformed("entry name truncated"));
        }
        let name = String::from_utf8(bytes[cursor..cursor + name_len].to_vec())
            .map_err(|_| SecretsError::Malformed("entry name is not valid UTF-8"))?;
        cursor += name_len;

        if cursor + ENTRY_HEADER_LEN > bytes.len() {
            return Err(SecretsError::Malformed("entry Argon2id header truncated"));
        }
        let m_cost_kib =
            u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().expect("4-byte slice"));
        let t_cost = u32::from_le_bytes(
            bytes[cursor + 4..cursor + 8]
                .try_into()
                .expect("4-byte slice"),
        );
        let p_cost = u32::from_le_bytes(
            bytes[cursor + 8..cursor + 12]
                .try_into()
                .expect("4-byte slice"),
        );
        let mut salt = [0u8; crate::SALT_LEN];
        salt.copy_from_slice(&bytes[cursor + 12..cursor + ENTRY_HEADER_LEN]);
        cursor += ENTRY_HEADER_LEN;
        let header = EntryHeader {
            m_cost_kib,
            t_cost,
            p_cost,
            salt,
        };

        if cursor + 4 > bytes.len() {
            return Err(SecretsError::Malformed("blob length truncated"));
        }
        let sealed_len =
            u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().expect("4-byte slice"))
                as usize;
        cursor += 4;

        if cursor + sealed_len > bytes.len() {
            return Err(SecretsError::Malformed("encrypted blob truncated"));
        }
        let sealed = bytes[cursor..cursor + sealed_len].to_vec();
        cursor += sealed_len;

        if entries
            .insert(name, StoredEntry { header, sealed })
            .is_some()
        {
            return Err(SecretsError::Malformed("duplicate entry name"));
        }
    }

    if cursor != bytes.len() {
        return Err(SecretsError::Malformed("extra bytes after the last entry"));
    }
    Ok(entries)
}

fn encode_v2(entries: &HashMap<String, StoredEntry>) -> Vec<u8> {
    let mut names: Vec<&String> = entries.keys().collect();
    names.sort_unstable();

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    #[allow(clippy::cast_possible_truncation)]
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());

    for name in names {
        let entry = &entries[name];
        #[allow(clippy::cast_possible_truncation)]
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&entry.header.to_bytes());
        #[allow(clippy::cast_possible_truncation)]
        out.extend_from_slice(&(entry.sealed.len() as u32).to_le_bytes());
        out.extend_from_slice(&entry.sealed);
    }
    out
}

#[derive(Deserialize, Default)]
struct LegacyRoot {
    remotes: HashMap<String, RemoteConfig>,
}

#[allow(clippy::indexing_slicing, clippy::expect_used)]
fn decode_legacy_v1(
    bytes: &[u8],
    password: &[u8],
) -> Result<HashMap<String, RemoteConfig>, SecretsError> {
    if bytes.len() < LEGACY_HEADER_LEN {
        return Err(SecretsError::Malformed("file too short"));
    }
    if &bytes[0..5] != MAGIC {
        return Err(SecretsError::Malformed("magic bytes missing"));
    }
    let version = bytes[5];
    if version != LEGACY_VERSION {
        return Err(SecretsError::UnsupportedVersion(version));
    }
    let m_cost_kib = u32::from_le_bytes(bytes[6..10].try_into().expect("4-byte slice"));
    let t_cost = u32::from_le_bytes(bytes[10..14].try_into().expect("4-byte slice"));
    let p_cost = u32::from_le_bytes(bytes[14..18].try_into().expect("4-byte slice"));
    let salt = &bytes[18..LEGACY_HEADER_LEN];
    let aad = &bytes[..LEGACY_HEADER_LEN];
    let sealed = &bytes[LEGACY_HEADER_LEN..];

    let params = KdfParams::from_untrusted(m_cost_kib, t_cost, p_cost)?;
    let kek = derive_kek(password, salt, params)?;
    let plaintext = zeroize::Zeroizing::new(open_with_aad(&kek, aad, sealed)?);
    let root: LegacyRoot = serde_json::from_slice(&plaintext)?;
    Ok(root.remotes)
}

fn tmp_path(dest: &Path) -> PathBuf {
    let file_name = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    dest.with_file_name(format!(
        ".{file_name}.cusec-tmp.{}.{nanos}",
        std::process::id()
    ))
}

#[cfg(unix)]
pub(crate) fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)
}

#[cfg(windows)]
pub(crate) fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)?;
    restrict_permissions_windows(path);
    Ok(())
}

#[cfg(windows)]
fn restrict_permissions_windows(path: &Path) {
    use winapi::um::winnt::{FILE_ALL_ACCESS, PSID};
    use windows_acl::acl::ACL;
    use windows_acl::helper::{current_user, name_to_sid, sid_to_string};

    let Some(path_str) = path.to_str() else {
        return;
    };
    let Some(user) = current_user() else {
        return;
    };
    let Ok(user_sid) = name_to_sid(&user, None) else {
        return;
    };
    let Ok(user_sid_string) = sid_to_string(user_sid.as_ptr() as PSID) else {
        return;
    };
    let Ok(mut acl) = ACL::from_file_path(path_str, false) else {
        return;
    };
    let to_remove: Vec<Vec<u16>> = acl
        .all()
        .map(|entries| {
            entries
                .into_iter()
                .filter(|entry| entry.string_sid != user_sid_string)
                .filter_map(|entry| entry.sid)
                .collect()
        })
        .unwrap_or_default();
    if acl
        .allow(user_sid.as_ptr() as PSID, false, FILE_ALL_ACCESS)
        .is_err()
    {
        return;
    }
    for sid in &to_remove {
        let _ = acl.remove(sid.as_ptr() as PSID, None, None);
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
}
