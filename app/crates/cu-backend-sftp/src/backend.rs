// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;
use std::sync::Mutex;

use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

use async_trait::async_trait;
use cu_backend::{
    BackendError, Capabilities, EntryKind, ListEntry, ObjPath, ObjStat, StorageBackend,
    StreamReader, StreamWriter,
};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, load_secret_key};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{OpenFlags, StatusCode};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _};

use crate::config::{CONNECT_TIMEOUT, SftpAuth, SftpConfig, client_config};
use crate::hostkey::{Handler, HostKeyInfo};
use crate::reader::SftpReader;
use crate::writer::SftpWriter;

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn map_sftp_err(e: russh_sftp::client::error::Error) -> BackendError {
    use russh_sftp::client::error::Error as E;
    match &e {
        E::Status(s) if s.status_code == StatusCode::NoSuchFile => BackendError::NotFound,
        E::Timeout => BackendError::Transient(format!("sftp: {e}")),
        _ => BackendError::Other(format!("sftp: {e}")),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn map_io_err(e: std::io::Error) -> BackendError {
    BackendError::Other(format!("sftp: I/O: {e}"))
}

struct Conn {
    handle: russh::client::Handle<Handler>,
    sftp: Arc<SftpSession>,
}

pub struct SftpBackend {
    cfg: SftpConfig,
    conn: RwLock<Arc<Conn>>,
    remote_root: String,
}

fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{nanos:x}.{}.{counter:x}", std::process::id())
}

fn tmp_path_for(dest: &str) -> String {
    const PREFIX_CHARS: usize = 48;
    let (dir, name) = dest.rfind('/').map_or(("", dest), |i| dest.split_at(i + 1));
    let prefix: String = name.chars().take(PREFIX_CHARS).collect();
    format!("{dir}{prefix}.cu-tmp.{}", unique_suffix())
}

fn parent_of(remote_path: &str) -> Option<&str> {
    remote_path.rfind('/').map(|i| &remote_path[..i])
}

async fn create_dir_all_str(sftp: &SftpSession, remote_path: &str) -> Result<(), BackendError> {
    let mut prefix = String::new();
    for segment in remote_path.split('/').filter(|s| !s.is_empty()) {
        if remote_path.starts_with('/') && prefix.is_empty() {
            prefix.push('/');
        }
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        prefix.push_str(segment);
        if let Err(e) = sftp.create_dir(prefix.clone()).await
            && !sftp.try_exists(prefix.clone()).await.unwrap_or(false)
        {
            return Err(map_sftp_err(e));
        }
    }
    Ok(())
}

async fn create_parent_of(sftp: &SftpSession, dest: &str) -> Result<(), BackendError> {
    if let Some(parent) = parent_of(dest)
        && !parent.is_empty()
    {
        create_dir_all_str(sftp, parent).await?;
    }
    Ok(())
}

fn remove_dir_all_str<'a>(
    sftp: &'a SftpSession,
    remote_path: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), BackendError>> + Send + 'a>> {
    Box::pin(async move {
        let entries = match sftp.read_dir(remote_path).await {
            Ok(entries) => entries,
            Err(e) if matches!(map_sftp_err(e.clone()), BackendError::NotFound) => {
                return Ok(());
            }
            Err(e) => return Err(map_sftp_err(e)),
        };
        for entry in entries {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child = format!("{remote_path}/{name}");
            if entry.file_type().is_dir() {
                remove_dir_all_str(sftp, &child).await?;
            } else {
                sftp.remove_file(&child).await.map_err(map_sftp_err)?;
            }
        }
        sftp.remove_dir(remote_path).await.map_err(map_sftp_err)
    })
}

async fn rename_overwriting(sftp: &SftpSession, from: &str, to: &str) -> Result<(), BackendError> {
    create_parent_of(sftp, to).await?;
    let first_err = match sftp.rename(from, to).await {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    if !sftp.try_exists(from).await.map_err(map_sftp_err)? {
        return Err(map_sftp_err(first_err));
    }
    let _ = sftp.remove_file(to).await;
    sftp.rename(from, to).await.map_err(map_sftp_err)
}

impl SftpBackend {
    pub async fn connect(cfg: &SftpConfig) -> Result<Self, BackendError> {
        let conn = Self::open_conn(cfg).await?;
        Ok(Self {
            cfg: cfg.clone(),
            conn: RwLock::new(Arc::new(conn)),
            remote_root: cfg.remote_root.trim_end_matches('/').to_string(),
        })
    }

    async fn open_conn(cfg: &SftpConfig) -> Result<Conn, BackendError> {
        let observed = Arc::new(Mutex::new(None));
        let handler = Handler {
            observed: Arc::clone(&observed),
        };
        let config = client_config();
        let mut handle = tokio::time::timeout(
            CONNECT_TIMEOUT,
            crate::config::connect(config, cfg.host.as_str(), cfg.port, handler),
        )
        .await
        .map_err(|_| BackendError::Transient("sftp: connect timeout".to_string()))??;

        let observed: Option<HostKeyInfo> = observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let observed = observed.ok_or_else(|| {
            BackendError::Other("sftp: no host key observed during the handshake".into())
        })?;
        if observed.fingerprint != cfg.expected_host_key_fingerprint {
            return Err(BackendError::Coded(
                cu_backend::Msg::new("SFTP_HOST_KEY_CHANGED")
                    .arg(&cfg.host)
                    .arg(cfg.port)
                    .arg(&cfg.expected_host_key_fingerprint)
                    .arg(&observed.fingerprint),
            ));
        }

        let auth_ok = match &cfg.auth {
            SftpAuth::Password(password) => handle
                .authenticate_password(cfg.username.clone(), password.clone())
                .await
                .map_err(|e| BackendError::Other(format!("sftp: authentication: {e}")))?
                .success(),
            SftpAuth::PrivateKey { path, passphrase } => {
                let key = load_secret_key(path, passphrase.as_deref())
                    .map_err(|e| BackendError::Other(format!("sftp: private key: {e}")))?;
                let key = PrivateKeyWithHashAlg::new(Arc::new(key), Some(HashAlg::Sha256));
                handle
                    .authenticate_publickey(cfg.username.clone(), key)
                    .await
                    .map_err(|e| BackendError::Other(format!("sftp: authentication: {e}")))?
                    .success()
            }
        };
        if !auth_ok {
            return Err(BackendError::Coded(
                cu_backend::Msg::new("SFTP_AUTH_REFUSED")
                    .arg(&cfg.username)
                    .arg(&cfg.host),
            ));
        }

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| BackendError::Other(format!("sftp: channel open: {e}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| BackendError::Other(format!("sftp: subsystem request: {e}")))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(map_sftp_err)?;
        Ok(Conn {
            handle,
            sftp: Arc::new(sftp),
        })
    }

    async fn current(&self) -> Arc<Conn> {
        Arc::clone(&*self.conn.read().await)
    }

    async fn reconnect(&self, stale: &Arc<Conn>) -> Result<(), BackendError> {
        let mut guard = self.conn.write().await;
        if Arc::ptr_eq(&guard, stale) {
            *guard = Arc::new(Self::open_conn(&self.cfg).await?);
        }
        Ok(())
    }

    async fn with_session<T, F, Fut>(&self, op: F) -> Result<T, BackendError>
    where
        F: Fn(Arc<SftpSession>) -> Fut,
        Fut: std::future::Future<Output = Result<T, BackendError>>,
    {
        let conn = self.current().await;
        match op(Arc::clone(&conn.sftp)).await {
            Err(e) if conn.handle.is_closed() || e.is_transient() => {
                self.reconnect(&conn).await?;
                op(Arc::clone(&self.current().await.sftp)).await
            }
            other => other,
        }
    }

    fn remote_path(&self, path: &ObjPath) -> String {
        if path.is_root() {
            if self.remote_root.is_empty() {
                ".".to_string()
            } else {
                self.remote_root.clone()
            }
        } else if self.remote_root.is_empty() {
            path.as_str().to_string()
        } else {
            format!("{}/{}", self.remote_root, path.as_str())
        }
    }

    async fn open_write_with_mode(
        &self,
        path: &ObjPath,
        exclusive: bool,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        let dest = self.remote_path(path);
        let tmp = tmp_path_for(&dest);
        let (sftp, file) = self
            .with_session(|sftp| {
                let (dest, tmp) = (dest.clone(), tmp.clone());
                async move {
                    create_parent_of(&sftp, &dest).await?;
                    let file = sftp
                        .open_with_flags(
                            &tmp,
                            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE,
                        )
                        .await
                        .map_err(map_sftp_err)?;
                    Ok((sftp, file))
                }
            })
            .await?;
        Ok(Box::new(SftpWriter::new(
            sftp,
            tmp,
            dest.clone(),
            format!("{dest}.cu-resume"),
            file,
            exclusive,
        )))
    }
}

#[async_trait]
impl StorageBackend for SftpBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            atomic_write: true,
            native_rename: true,
            conditional_create: true,
            random_read: true,
            stale_path_cache: false,
        }
    }

    async fn exists(&self, path: &ObjPath) -> Result<bool, BackendError> {
        let p = self.remote_path(path);
        self.with_session(|sftp| {
            let p = p.clone();
            async move { sftp.try_exists(p).await.map_err(map_sftp_err) }
        })
        .await
    }

    async fn get(&self, path: &ObjPath) -> Result<Vec<u8>, BackendError> {
        let p = self.remote_path(path);
        self.with_session(|sftp| {
            let p = p.clone();
            async move { sftp.read(p).await.map_err(map_sftp_err) }
        })
        .await
    }

    async fn get_range(
        &self,
        path: &ObjPath,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        let p = self.remote_path(path);
        self.with_session(|sftp| {
            let p = p.clone();
            async move {
                let mut file = sftp.open(p).await.map_err(map_sftp_err)?;
                file.seek(std::io::SeekFrom::Start(offset))
                    .await
                    .map_err(map_io_err)?;
                let want = usize::try_from(len).unwrap_or(usize::MAX);
                let mut buf = vec![0u8; want];
                let mut filled = 0;
                while filled < want {
                    let n = file.read(&mut buf[filled..]).await.map_err(map_io_err)?;
                    if n == 0 {
                        break;
                    }
                    filled += n;
                }
                buf.truncate(filled);
                Ok(buf)
            }
        })
        .await
    }

    async fn stat(&self, path: &ObjPath) -> Result<ObjStat, BackendError> {
        let p = self.remote_path(path);
        self.with_session(|sftp| {
            let p = p.clone();
            async move {
                let meta = sftp.metadata(p).await.map_err(map_sftp_err)?;
                Ok(ObjStat {
                    size: meta.size.unwrap_or(0),
                })
            }
        })
        .await
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        let dest = self.remote_path(path);
        self.with_session(|sftp| {
            let dest = dest.clone();
            async move {
                create_parent_of(&sftp, &dest).await?;
                let tmp = tmp_path_for(&dest);
                let written = async {
                    let mut file = sftp.create(&tmp).await.map_err(map_sftp_err)?;
                    file.write_all(data).await.map_err(map_io_err)?;
                    file.close().await.map_err(map_io_err)?;
                    rename_overwriting(&sftp, &tmp, &dest).await
                }
                .await;
                if written.is_err() {
                    let _ = sftp.remove_file(&tmp).await;
                }
                written
            }
        })
        .await
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        let p = self.remote_path(path);
        let file = self
            .with_session(|sftp| {
                let p = p.clone();
                async move { sftp.open(p).await.map_err(map_sftp_err) }
            })
            .await?;
        Ok(Box::new(SftpReader::new(file)))
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        self.open_write_with_mode(path, false).await
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        self.open_write_with_mode(path, true).await
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        let p = self.remote_path(path);
        self.with_session(|sftp| {
            let p = p.clone();
            async move {
                match sftp.remove_file(p).await {
                    Ok(()) => Ok(()),
                    Err(e) if matches!(map_sftp_err(e.clone()), BackendError::NotFound) => Ok(()),
                    Err(e) => Err(map_sftp_err(e)),
                }
            }
        })
        .await
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        let p = self.remote_path(dir);
        let entries = self
            .with_session(|sftp| {
                let p = p.clone();
                async move { sftp.read_dir(p).await.map_err(map_sftp_err) }
            })
            .await?;
        Ok(entries
            .into_iter()
            .filter(|e| e.file_name() != "." && e.file_name() != "..")
            .map(|e| {
                let is_dir = e.file_type().is_dir();
                ListEntry {
                    name: e.file_name(),
                    kind: if is_dir {
                        EntryKind::Directory
                    } else {
                        EntryKind::File
                    },
                    size: if is_dir { None } else { e.metadata().size },
                    modified: if is_dir {
                        None
                    } else {
                        e.metadata().mtime.map(i64::from)
                    },
                }
            })
            .collect())
    }

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        let p = self.remote_path(dir);
        self.with_session(|sftp| {
            let p = p.clone();
            async move { create_dir_all_str(&sftp, &p).await }
        })
        .await
    }

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        _from_kind: EntryKind,
    ) -> Result<(), BackendError> {
        let (f, t) = (self.remote_path(from), self.remote_path(to));
        self.with_session(|sftp| {
            let (f, t) = (f.clone(), t.clone());
            async move { rename_overwriting(&sftp, &f, &t).await }
        })
        .await
    }

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        let p = self.remote_path(dir);
        self.with_session(|sftp| {
            let p = p.clone();
            async move { remove_dir_all_str(&sftp, &p).await }
        })
        .await
    }

    async fn load_resume_prologue(
        &self,
        path: &ObjPath,
        content_hash: Option<[u8; 32]>,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        let dest = self.remote_path(path);
        let resume_path = format!("{dest}.cu-resume");
        let sftp = Arc::clone(&self.current().await.sftp);
        let Ok(sidecar) = sftp.read(&resume_path).await else {
            return Ok(None);
        };
        let Some(expected_hash) = content_hash else {
            return Ok(None);
        };
        if sidecar.len() < 32 || sidecar[..32] != expected_hash {
            let _ = sftp.remove_file(&resume_path).await;
            return Ok(None);
        }
        let prologue = sidecar[32..].to_vec();
        Ok(Some(prologue))
    }

    async fn store_resume_prologue(
        &self,
        path: &ObjPath,
        bytes: &[u8],
        content_hash: Option<[u8; 32]>,
    ) -> Result<(), BackendError> {
        let dest = self.remote_path(path);
        let resume_path = format!("{dest}.cu-resume");
        let hash = content_hash.unwrap_or_else(fresh_marker);
        let mut sidecar = Vec::with_capacity(32 + bytes.len());
        sidecar.extend_from_slice(&hash);
        sidecar.extend_from_slice(bytes);
        self.with_session(|sftp| {
            let (dest, resume_path, sidecar) = (dest.clone(), resume_path.clone(), &sidecar);
            async move {
                create_parent_of(&sftp, &dest).await?;
                let mut file = sftp.create(&resume_path).await.map_err(map_sftp_err)?;
                file.write_all(sidecar).await.map_err(map_io_err)?;
                file.close().await.map_err(map_io_err)
            }
        })
        .await
    }
}

fn fresh_marker() -> [u8; 32] {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&nanos.to_le_bytes());
    out[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    out
}
