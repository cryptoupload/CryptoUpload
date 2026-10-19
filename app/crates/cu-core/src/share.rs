// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::{Path, PathBuf};

use cu_crypto::kdf::KdfParams;
use cu_format::content::{
    CIPHERTEXT_CHUNK_LEN, ContentDecryptor, ContentEncryptor, PLAINTEXT_CHUNK_LEN,
};
use cu_format::inbox::{self, Prologue};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::error::CoreError;
use crate::inbox::sanitize_filename;
use crate::vault::{NodeKind, Vault};

pub const SHARE_EXTENSION: &str = "cushare";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareInfo {
    pub original_name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedShare {
    pub path: PathBuf,
    pub original_name: String,
    pub bytes: u64,
}

fn package_kdf_params() -> KdfParams {
    KdfParams::sensitive()
}

fn part_path(final_path: &Path) -> Result<PathBuf, CoreError> {
    let mut nonce = [0u8; 6];
    cu_crypto::random::fill_random(&mut nonce).map_err(|_| CoreError::RandomUnavailable)?;
    let suffix = nonce.iter().fold(String::new(), |mut acc, b| {
        use std::fmt::Write;
        let _ = write!(acc, "{b:02x}");
        acc
    });
    let name = final_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("package");
    Ok(final_path.with_file_name(format!(".{name}.{suffix}.part")))
}

async fn read_full<R: AsyncRead + Unpin>(src: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0usize;
    while filled < buf.len() {
        let n = src.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

impl Vault {
    pub async fn share_file(
        &self,
        vault_src: &str,
        out: &Path,
        password: &[u8],
        overwrite: bool,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<ShareInfo, CoreError> {
        if password.is_empty() {
            return Err(CoreError::Share(Msg::new("SHARE_EMPTY_PASSWORD")));
        }
        crate::vault::check_new_password(password)?;
        if !overwrite && tokio::fs::try_exists(out).await? {
            return Err(CoreError::AlreadyExists);
        }

        let trimmed = vault_src.trim_matches('/');
        let (parent, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
        let entry = self
            .list(parent)
            .await?
            .into_iter()
            .find(|e| e.name == name && e.kind == NodeKind::File)
            .ok_or(CoreError::NotFound)?;
        let size = match entry.size {
            Some(size) => size,
            None => self
                .fs
                .open_read_at(self.config(), vault_src)
                .await?
                .plaintext_len(),
        };
        let prologue = inbox::encode_prologue(&Prologue {
            original_name: entry.name.clone(),
            size,
            mtime_ms: entry.modified.map_or(0, |secs| secs.saturating_mul(1000)),
            depositor_label: String::new(),
        })?;

        let (header, content_key, content_nonce) = seal_header_blocking(password).await?;

        let part = part_path(out)?;
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent).await?;
        }
        let result = self
            .write_package(
                trimmed,
                &part,
                &header,
                content_key,
                content_nonce,
                prologue,
                on_bytes,
            )
            .await;
        match result {
            Ok(bytes) => {
                if let Err(e) = tokio::fs::rename(&part, out).await {
                    let _ = tokio::fs::remove_file(&part).await;
                    return Err(e.into());
                }
                Ok(ShareInfo {
                    original_name: entry.name,
                    bytes,
                })
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                Err(e)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_package(
        &self,
        vault_src: &str,
        part: &Path,
        header: &[u8],
        content_key: cu_crypto::aead::ContentKey,
        content_nonce: [u8; cu_crypto::aead::NONCE_LEN],
        prologue: Vec<u8>,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<u64, CoreError> {
        let mut file = tokio::fs::File::create(part).await?;
        file.write_all(header).await?;

        let (mut tx, mut rx) = tokio::io::duplex(2 * PLAINTEXT_CHUNK_LEN);
        let fs = self.fs();
        let config = self.config();
        let producer = async move {
            let result = fs.read_file(config, vault_src, &mut tx, on_bytes).await;
            let _ = tx.shutdown().await;
            drop(tx);
            result
        };
        let consumer = async {
            let mut encryptor = ContentEncryptor::new(content_key, content_nonce);
            let mut pending = prologue;
            let mut content_bytes = 0u64;
            loop {
                let mut buf = vec![0u8; PLAINTEXT_CHUNK_LEN];
                let mut filled = 0usize;
                if !pending.is_empty() {
                    let take = pending.len().min(buf.len());
                    buf[..take].copy_from_slice(&pending[..take]);
                    pending.drain(..take);
                    filled = take;
                }
                if filled < buf.len() {
                    let before = filled;
                    filled += read_full(&mut rx, &mut buf[filled..]).await?;
                    content_bytes += (filled - before) as u64;
                }
                if filled == 0 {
                    break;
                }
                file.write_all(&encryptor.seal_next(&buf[..filled])?)
                    .await?;
                if filled < buf.len() {
                    break;
                }
            }
            if let Some(eof) = encryptor.finish()? {
                file.write_all(&eof).await?;
            }
            file.flush().await?;
            file.sync_all().await?;
            Ok::<u64, CoreError>(content_bytes)
        };
        let (read_result, write_result) = tokio::join!(producer, consumer);
        let content_bytes = write_result?;
        read_result?;
        Ok(content_bytes)
    }
}

pub async fn open_share(
    package: &Path,
    password: &[u8],
    dest_dir: &Path,
    on_bytes: &mut (dyn FnMut(u64) + Send),
) -> Result<OpenedShare, CoreError> {
    let mut src = tokio::fs::File::open(package).await?;
    let mut header = vec![0u8; cu_format::share::HEADER_LEN];
    let got = read_full(&mut src, &mut header).await?;
    header.truncate(got);
    let (content_key, content_nonce) = open_header_blocking(header, password).await?;
    let mut decryptor = ContentDecryptor::new(content_key, content_nonce);

    tokio::fs::create_dir_all(dest_dir).await?;
    let mut state = PartState::default();

    let result = decrypt_into_part(&mut src, &mut decryptor, dest_dir, on_bytes, &mut state).await;
    let PartState {
        prologue,
        mut part,
        bytes,
    } = state;

    let original_name = prologue
        .as_ref()
        .map(|(p, _)| p.original_name.clone())
        .unwrap_or_default();
    match result {
        Ok(()) => {
            let Some((part_file, file)) = part.take() else {
                return Err(CoreError::Share(Msg::new("SHARE_NO_HEADER")));
            };
            drop(file);
            let path = match claim_and_rename(&part_file, dest_dir, &original_name).await {
                Ok(path) => path,
                Err(e) => {
                    let _ = tokio::fs::remove_file(&part_file).await;
                    return Err(e);
                }
            };
            Ok(OpenedShare {
                path,
                original_name,
                bytes,
            })
        }
        Err(e) => {
            if let Some((part_file, file)) = part.take() {
                drop(file);
                let _ = tokio::fs::remove_file(&part_file).await;
            }
            Err(e)
        }
    }
}

#[derive(Default)]
struct PartState {
    prologue: Option<(Prologue, usize)>,
    part: Option<(PathBuf, tokio::fs::File)>,
    bytes: u64,
}

async fn decrypt_into_part(
    src: &mut tokio::fs::File,
    decryptor: &mut ContentDecryptor,
    dest_dir: &Path,
    on_bytes: &mut (dyn FnMut(u64) + Send),
    state: &mut PartState,
) -> Result<(), CoreError> {
    let mut head: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; CIPHERTEXT_CHUNK_LEN];
    while !decryptor.is_done() {
        let n = read_full(src, &mut chunk).await?;
        if n == 0 {
            return Err(CoreError::Share(Msg::new("SHARE_TRUNCATED")));
        }
        let plain = decryptor.feed_chunk(&chunk[..n])?;
        let mut content: &[u8] = &plain;
        let rest;
        if state.prologue.is_none() {
            head.extend_from_slice(&plain);
            state.prologue = inbox::decode_prologue(&head)?;
            let Some((_, consumed)) = &state.prologue else {
                continue;
            };
            rest = head.split_off(*consumed);
            let part_file = part_path(&dest_dir.join(".cushare"))?;
            let file = tokio::fs::File::create(&part_file).await?;
            state.part = Some((part_file, file));
            content = &rest;
        }
        if !content.is_empty() {
            if let Some((_, file)) = state.part.as_mut() {
                file.write_all(content).await?;
            }
            state.bytes += content.len() as u64;
            on_bytes(state.bytes);
        }
    }
    let mut extra = [0u8; 1];
    if read_full(src, &mut extra).await? != 0 {
        return Err(CoreError::Share(Msg::new("SHARE_TRAILING_BYTES")));
    }
    let Some((_, file)) = state.part.as_mut() else {
        return Err(CoreError::Share(Msg::new("SHARE_NO_HEADER")));
    };
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}

fn local_safe_name(original_name: &str) -> String {
    let cleaned: String = sanitize_filename(original_name)
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "shared-file".to_string()
    } else {
        trimmed.to_string()
    }
}

fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot < name.len() - 1 => {
            format!("{} ({n}){}", &name[..dot], &name[dot..])
        }
        _ => format!("{name} ({n})"),
    }
}

const MAX_KEEP_BOTH_ATTEMPTS: u32 = 1000;

async fn claim_and_rename(
    part_file: &Path,
    dest_dir: &Path,
    original_name: &str,
) -> Result<PathBuf, CoreError> {
    let base = local_safe_name(original_name);
    for attempt in 1..=MAX_KEEP_BOTH_ATTEMPTS {
        let name = if attempt == 1 {
            base.clone()
        } else {
            numbered(&base, attempt)
        };
        let target = dest_dir.join(&name);
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .await
        {
            Ok(placeholder) => {
                drop(placeholder);
                if let Err(e) = tokio::fs::rename(part_file, &target).await {
                    let _ = tokio::fs::remove_file(&target).await;
                    return Err(e.into());
                }
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err(CoreError::AlreadyExists)
}

async fn seal_header_blocking(
    password: &[u8],
) -> Result<
    (
        Vec<u8>,
        cu_crypto::aead::ContentKey,
        [u8; cu_crypto::aead::NONCE_LEN],
    ),
    CoreError,
> {
    let password = password.to_vec();
    tokio::task::spawn_blocking(move || {
        cu_format::share::seal_header(&password, package_kdf_params())
    })
    .await
    .map_err(|e| CoreError::Share(Msg::new("SHARE_KEY_INTERRUPTED").arg(e)))?
    .map_err(CoreError::from)
}

async fn open_header_blocking(
    header: Vec<u8>,
    password: &[u8],
) -> Result<
    (
        cu_crypto::aead::ContentKey,
        [u8; cu_crypto::aead::NONCE_LEN],
    ),
    CoreError,
> {
    let password = password.to_vec();
    tokio::task::spawn_blocking(move || cu_format::share::open_header(&header, &password))
        .await
        .map_err(|e| CoreError::Share(Msg::new("SHARE_KEY_INTERRUPTED").arg(e)))?
        .map_err(CoreError::from)
}
