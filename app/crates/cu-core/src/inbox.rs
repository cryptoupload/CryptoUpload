// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg};
use std::path::Path;
use std::time::UNIX_EPOCH;

use cu_backend::{BackendError, EntryKind, ObjPath, StorageBackend, StreamReader};
use cu_crypto::x25519::StaticKeypair;
use cu_format::config::VaultConfig;
use cu_format::content::{
    CIPHERTEXT_CHUNK_LEN, ContentDecryptor, ContentEncryptor, PLAINTEXT_CHUNK_LEN,
};
use cu_format::inbox::{self, Prologue};
use cu_vfs::VaultFs;
use tokio::io::AsyncReadExt;

use crate::error::CoreError;
use crate::remote::{RefreshTokenSink, RemoteConfig};

pub const INBOX_DIR: &str = ".cryptoupload/inbox";

pub const DEFAULT_MAX_DEPOSIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct CollectLimits {
    pub max_deposit_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for CollectLimits {
    fn default() -> Self {
        Self {
            max_deposit_bytes: DEFAULT_MAX_DEPOSIT_BYTES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InboxListing {
    pub deposit_name: String,
    pub encrypted_size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CollectedDeposit {
    pub deposit_name: String,
    pub vault_path: String,
    pub original_name: String,
    pub depositor_label: String,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct SkippedDeposit {
    pub deposit_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct CollectReport {
    pub collected: Vec<CollectedDeposit>,
    pub skipped: Vec<SkippedDeposit>,
}

pub async fn deposit_via_remote(
    cfg: &RemoteConfig,
    remote_state_db: &Path,
    inbox_public_key: &[u8; 32],
    local_src: &Path,
    depositor_label: &str,
    refresh_sink: Option<RefreshTokenSink>,
) -> Result<(), CoreError> {
    let backend =
        crate::remote::build_backend_with_refresh_sink(cfg, remote_state_db, None, refresh_sink)
            .await?;
    deposit_to_backend(
        backend.as_ref(),
        inbox_public_key,
        local_src,
        depositor_label,
    )
    .await
}

pub async fn deposit_to_backend(
    backend: &dyn StorageBackend,
    inbox_public_key: &[u8; 32],
    local_src: &Path,
    depositor_label: &str,
) -> Result<(), CoreError> {
    let metadata = tokio::fs::metadata(local_src).await?;
    let size = metadata.len();
    let mtime_ms = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    let original_name = local_src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let prologue = Prologue {
        original_name,
        size,
        mtime_ms,
        depositor_label: depositor_label.to_string(),
    };
    let prologue_bytes = inbox::encode_prologue(&prologue)?;

    let (header_bytes, content_key, content_nonce) = inbox::seal_header(inbox_public_key)?;

    let inbox_dir = ObjPath::new(INBOX_DIR)?;
    backend.create_dir_all(&inbox_dir).await?;
    let dest = inbox_dir.join(&random_deposit_name()?)?;

    let mut writer = backend.open_write(&dest).await?;
    writer.write_chunk(&header_bytes).await?;

    let mut encryptor = ContentEncryptor::new(content_key, content_nonce);
    let mut pending = prologue_bytes;
    let mut src = tokio::fs::File::open(local_src).await?;
    loop {
        let mut buf = vec![0u8; PLAINTEXT_CHUNK_LEN];
        let mut filled = 0usize;
        if !pending.is_empty() {
            let take = pending.len().min(buf.len());
            buf[..take].copy_from_slice(&pending[..take]);
            pending.drain(..take);
            filled = take;
        }
        while filled < buf.len() {
            let n = src.read(&mut buf[filled..]).await?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        writer
            .write_chunk(&encryptor.seal_next(&buf[..filled])?)
            .await?;
        if filled < buf.len() {
            break;
        }
    }
    if let Some(eof) = encryptor.finish()? {
        writer.write_chunk(&eof).await?;
    }
    writer.commit().await?;
    Ok(())
}

fn random_deposit_name() -> Result<String, CoreError> {
    let mut buf = [0u8; 16];
    cu_crypto::random::fill_random(&mut buf).map_err(|_| CoreError::RandomUnavailable)?;
    Ok(hex(&buf))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

pub(crate) async fn list_raw(backend: &dyn StorageBackend) -> Result<Vec<InboxListing>, CoreError> {
    let inbox_dir = ObjPath::new(INBOX_DIR)?;
    let entries = match backend.list(&inbox_dir).await {
        Ok(entries) => entries,
        Err(BackendError::NotFound) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(entries
        .into_iter()
        .filter(|e| e.kind == EntryKind::File)
        .map(|e| InboxListing {
            deposit_name: e.name,
            encrypted_size: e.size,
        })
        .collect())
}

pub(crate) async fn collect_one(
    fs: &VaultFs,
    config: &VaultConfig,
    backend: &dyn StorageBackend,
    deposit_path: &ObjPath,
    inbox_keypair: &StaticKeypair,
    remaining_budget: u64,
) -> Result<CollectedDeposit, Msg> {
    let mut reader = backend.open_read(deposit_path).await.map_err(|e| e.msg())?;
    let header_bytes = reader
        .read_chunk(cu_format::inbox::HEADER_LEN)
        .await
        .map_err(|e| e.msg())?;
    let (content_key, content_nonce) =
        cu_format::inbox::open_header(&header_bytes, inbox_keypair).map_err(|e| e.msg())?;
    let mut stream = DepositStream {
        reader: Some(reader),
        decryptor: ContentDecryptor::new(content_key, content_nonce),
        finished: false,
    };

    let mut head = zeroize::Zeroizing::new(Vec::new());
    let (prologue, first) = loop {
        let Some(piece) = stream.next_piece().await? else {
            return Err(Msg::new("INBOX_NO_PROLOGUE"));
        };
        head.extend_from_slice(&piece);
        if let Some((prologue, consumed)) = inbox::decode_prologue(&head).map_err(|e| e.msg())? {
            let rest = head.get(consumed..).unwrap_or_default().to_vec();
            break (prologue, rest);
        }
    };
    if first.len() as u64 > remaining_budget {
        return Err(Msg::new("INBOX_TOO_LARGE").arg(remaining_budget));
    }

    let safe_name = sanitize_filename(&prologue.original_name);
    let safe_label = sanitize_label(&prologue.depositor_label);
    let base_path = format!("Inbox/{safe_label}/{safe_name}");
    let dest_path = keep_both_path(fs, config, &base_path)
        .await
        .map_err(|e| e.msg())?;

    let mut source = DepositReader {
        content_bytes: first.len() as u64,
        buf: zeroize::Zeroizing::new(first),
        pos: 0,
        stream,
        pending: None,
        budget: remaining_budget,
        failure: None,
    };
    let written = fs
        .write_file(config, &dest_path, &mut source, None, &mut |_| {})
        .await;
    if let Some(reason) = source.failure.take() {
        return Err(reason);
    }
    written.map_err(|e| e.msg())?;

    Ok(CollectedDeposit {
        deposit_name: String::new(),
        vault_path: dest_path,
        original_name: prologue.original_name,
        depositor_label: prologue.depositor_label,
        bytes: source.content_bytes,
    })
}

struct DepositStream {
    reader: Option<Box<dyn StreamReader>>,
    decryptor: ContentDecryptor,
    finished: bool,
}

impl DepositStream {
    fn want(&self) -> usize {
        if self.decryptor.is_done() {
            1
        } else {
            CIPHERTEXT_CHUNK_LEN
        }
    }

    fn accept(&mut self, chunk: &[u8]) -> Result<Option<Vec<u8>>, Msg> {
        if self.decryptor.is_done() {
            if !chunk.is_empty() {
                return Err(Msg::new("INBOX_TRAILING_BYTES"));
            }
            self.finished = true;
            return Ok(None);
        }
        if chunk.is_empty() {
            return Err(Msg::new("INBOX_TRUNCATED"));
        }
        self.decryptor
            .feed_chunk(chunk)
            .map(Some)
            .map_err(|e| e.msg())
    }

    async fn next_piece(&mut self) -> Result<Option<Vec<u8>>, Msg> {
        if self.finished {
            return Ok(None);
        }
        let want = self.want();
        let reader = self
            .reader
            .as_mut()
            .ok_or_else(|| Msg::new("INBOX_TRUNCATED"))?;
        let chunk = reader.read_chunk(want).await.map_err(|e| e.msg())?;
        self.accept(&chunk)
    }
}

type PendingRead = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = (Box<dyn StreamReader>, Result<Vec<u8>, BackendError>)>
            + Send,
    >,
>;

struct DepositReader {
    stream: DepositStream,
    pending: Option<PendingRead>,
    buf: zeroize::Zeroizing<Vec<u8>>,
    pos: usize,
    content_bytes: u64,
    budget: u64,
    failure: Option<Msg>,
}

impl DepositReader {
    fn fail(&mut self, reason: Msg) -> std::io::Error {
        let err = std::io::Error::other(reason.code());
        self.failure.get_or_insert(reason);
        err
    }
}

impl tokio::io::AsyncRead for DepositReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        out: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::task::Poll;
        let this = self.get_mut();
        loop {
            if let Some(reason) = &this.failure {
                return Poll::Ready(Err(std::io::Error::other(reason.code())));
            }
            if let Some(left) = this.buf.get(this.pos..).filter(|b| !b.is_empty()) {
                let n = left.len().min(out.remaining());
                out.put_slice(left.get(..n).unwrap_or_default());
                this.pos += n;
                return Poll::Ready(Ok(()));
            }
            if this.stream.finished {
                return Poll::Ready(Ok(()));
            }
            if this.pending.is_none() {
                let Some(mut reader) = this.stream.reader.take() else {
                    return Poll::Ready(Err(this.fail(Msg::new("INBOX_TRUNCATED"))));
                };
                let want = this.stream.want();
                this.pending = Some(Box::pin(async move {
                    let res = reader.read_chunk(want).await;
                    (reader, res)
                }));
            }
            let Some(pending) = this.pending.as_mut() else {
                continue;
            };
            let Poll::Ready((reader, res)) = pending.as_mut().poll(cx) else {
                return Poll::Pending;
            };
            this.pending = None;
            this.stream.reader = Some(reader);
            let piece = match res
                .map_err(|e| e.msg())
                .and_then(|chunk| this.stream.accept(&chunk))
            {
                Ok(Some(piece)) => piece,
                Ok(None) => continue,
                Err(reason) => return Poll::Ready(Err(this.fail(reason))),
            };
            this.content_bytes += piece.len() as u64;
            if this.content_bytes > this.budget {
                let reason = Msg::new("INBOX_TOO_LARGE").arg(this.budget);
                return Poll::Ready(Err(this.fail(reason)));
            }
            this.buf = zeroize::Zeroizing::new(piece);
            this.pos = 0;
        }
    }
}

const MAX_KEEP_BOTH_ATTEMPTS: u32 = 50;

async fn keep_both_path(
    fs: &VaultFs,
    config: &VaultConfig,
    base_path: &str,
) -> Result<String, cu_vfs::VfsError> {
    let (dir, filename) = match base_path.rsplit_once('/') {
        Some((d, f)) => (format!("{d}/"), f),
        None => (String::new(), base_path),
    };
    let (stem, ext) = match filename.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, Some(e)),
        _ => (filename, None),
    };
    for attempt in 1..=MAX_KEEP_BOTH_ATTEMPTS {
        let candidate_name = if attempt == 1 {
            filename.to_string()
        } else if let Some(e) = ext {
            format!("{stem} ({attempt}).{e}")
        } else {
            format!("{stem} ({attempt})")
        };
        let candidate = format!("{dir}{candidate_name}");
        if !fs.exists(config, &candidate).await? {
            return Ok(candidate);
        }
    }
    Err(cu_vfs::VfsError::AlreadyExists)
}

pub(crate) fn sanitize_filename(original_name: &str) -> String {
    let last_segment = original_name
        .split(['/', '\\'])
        .rfind(|s| !s.is_empty() && *s != "." && *s != "..")
        .unwrap_or("")
        .trim();
    if last_segment.is_empty() {
        "deposit".to_string()
    } else {
        last_segment.chars().take(200).collect()
    }
}

fn sanitize_label(depositor_label: &str) -> String {
    let cleaned: String = depositor_label
        .trim()
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "Unknown".to_string()
    } else {
        cleaned.chars().take(100).collect()
    }
}
