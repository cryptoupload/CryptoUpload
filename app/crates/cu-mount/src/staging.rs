// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use cu_crypto::aead::{self, CIPHERTEXT_CHUNK, ContentKey, NONCE_LEN, PLAINTEXT_CHUNK};
use cu_format::config::VaultConfig;
use cu_format::error::FormatError;
use cu_format::header::{self, HEADER_LEN};
use cu_format::keys;
use tokio::io::{AsyncRead, ReadBuf};

const PT: u64 = PLAINTEXT_CHUNK as u64;
const CT: u64 = CIPHERTEXT_CHUNK as u64;
const HDR: u64 = HEADER_LEN as u64;

#[derive(Debug)]
pub enum StagingError {
    Io(io::Error),
    Format(FormatError),
}

impl Coded for StagingError {
    fn msg(&self) -> Msg {
        match self {
            Self::Io(e) => Msg::new("STAGING_IO").arg(e),
            Self::Format(e) => e.msg(),
        }
    }
}

display_as_msg!(StagingError);

impl std::error::Error for StagingError {}

impl From<io::Error> for StagingError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<FormatError> for StagingError {
    fn from(e: FormatError) -> Self {
        Self::Format(e)
    }
}

impl From<cu_crypto::CryptoError> for StagingError {
    fn from(e: cu_crypto::CryptoError) -> Self {
        match e {
            cu_crypto::CryptoError::DecryptFailed => Self::Format(FormatError::DecryptFailed),
            _ => Self::Io(io::Error::other("staging: cryptographic error")),
        }
    }
}

struct Inner {
    file: File,
    len: u64,
    key: ContentKey,
    header_nonce: [u8; NONCE_LEN],
}

pub struct StagingFile {
    inner: Mutex<Inner>,
}

fn slot_offset(index: u64) -> u64 {
    HDR + index * CT
}

impl Inner {
    fn read_slot(&mut self, index: u64) -> Result<Vec<u8>, StagingError> {
        let mut buf = vec![0u8; CIPHERTEXT_CHUNK];
        self.file.seek(SeekFrom::Start(slot_offset(index)))?;
        let mut filled = 0usize;
        while let Some(rest) = buf.get_mut(filled..) {
            if rest.is_empty() {
                break;
            }
            let n = self.file.read(rest)?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 || buf.iter().take(NONCE_LEN).all(|b| *b == 0) {
            return Ok(vec![0u8; PLAINTEXT_CHUNK]);
        }
        if filled < CIPHERTEXT_CHUNK {
            return Err(StagingError::Format(FormatError::DecryptFailed));
        }
        Ok(aead::open_chunk(
            &self.key,
            index,
            &self.header_nonce,
            &buf,
        )?)
    }

    fn write_slot(&mut self, index: u64, plain: &[u8]) -> Result<(), StagingError> {
        let sealed = aead::seal_chunk(&self.key, index, &self.header_nonce, plain)?;
        self.file.seek(SeekFrom::Start(slot_offset(index)))?;
        self.file.write_all(&sealed)?;
        Ok(())
    }
}

impl StagingFile {
    pub fn create(path: &Path, config: &VaultConfig) -> Result<Self, StagingError> {
        let header_key = keys::header_key(config.latest_seed(), config.kdf_salt())?;
        let (header_bytes, key, header_nonce) = header::seal(&header_key, config.latest_seed_id())?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&header_bytes)?;
        Ok(Self {
            inner: Mutex::new(Inner {
                file,
                len: 0,
                key,
                header_nonce,
            }),
        })
    }

    pub fn open(path: &Path, config: &VaultConfig, len: u64) -> Result<Self, StagingError> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut header_bytes = vec![0u8; HEADER_LEN];
        file.read_exact(&mut header_bytes)?;
        let salt = *config.kdf_salt();
        let (key, header_nonce, _) = header::open(
            &header_bytes,
            |id| config.seed(id).cloned(),
            |seed| keys::header_key(seed, &salt),
        )?;
        Ok(Self {
            inner: Mutex::new(Inner {
                file,
                len,
                key,
                header_nonce,
            }),
        })
    }

    #[must_use]
    #[allow(clippy::unwrap_used)]
    pub fn len(&self) -> u64 {
        self.inner.lock().unwrap().len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[allow(clippy::unwrap_used)]
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, StagingError> {
        let mut inner = self.inner.lock().unwrap();
        if offset >= inner.len || buf.is_empty() {
            return Ok(0);
        }
        let want = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        let end = inner.len.min(offset.saturating_add(want));
        let mut pos = offset;
        let mut done = 0usize;
        while pos < end {
            let index = pos / PT;
            let within = usize::try_from(pos - index * PT).unwrap_or(0);
            let chunk = inner.read_slot(index)?;
            let avail = chunk.get(within..).unwrap_or(&[]);
            let take = avail
                .len()
                .min(usize::try_from(end - pos).unwrap_or(usize::MAX));
            let (Some(dst), Some(src)) = (buf.get_mut(done..done + take), avail.get(..take)) else {
                break;
            };
            dst.copy_from_slice(src);
            done += take;
            pos += take as u64;
        }
        Ok(done)
    }

    #[allow(clippy::unwrap_used)]
    pub fn write_at(&self, offset: u64, data: &[u8]) -> Result<(), StagingError> {
        if data.is_empty() {
            return Ok(());
        }
        let mut inner = self.inner.lock().unwrap();
        let end = offset + data.len() as u64;
        if end > inner.len {
            inner.len = end;
        }
        let mut pos = offset;
        let mut done = 0usize;
        while pos < end {
            let index = pos / PT;
            let within = usize::try_from(pos - index * PT).unwrap_or(0);
            let take = (PLAINTEXT_CHUNK - within).min(data.len() - done);
            let mut chunk = if within == 0 && take == PLAINTEXT_CHUNK {
                vec![0u8; PLAINTEXT_CHUNK]
            } else {
                inner.read_slot(index)?
            };
            let (Some(dst), Some(src)) = (
                chunk.get_mut(within..within + take),
                data.get(done..done + take),
            ) else {
                return Err(StagingError::Io(io::Error::other(
                    "staging: inconsistent offset",
                )));
            };
            dst.copy_from_slice(src);
            inner.write_slot(index, &chunk)?;
            done += take;
            pos += take as u64;
        }
        Ok(())
    }

    #[allow(clippy::unwrap_used)]
    pub fn set_len(&self, new_len: u64) -> Result<(), StagingError> {
        let mut inner = self.inner.lock().unwrap();
        if new_len >= inner.len {
            inner.len = new_len;
            return Ok(());
        }
        inner.len = new_len;
        let last = new_len / PT;
        let tail = usize::try_from(new_len % PT).unwrap_or(0);
        if tail == 0 {
            inner.file.set_len(slot_offset(last))?;
        } else {
            let mut chunk = inner.read_slot(last)?;
            if let Some(dead) = chunk.get_mut(tail..) {
                dead.fill(0);
            }
            inner.write_slot(last, &chunk)?;
            inner.file.set_len(slot_offset(last + 1))?;
        }
        Ok(())
    }

    #[allow(clippy::unwrap_used)]
    pub fn fsync(&self) -> Result<(), StagingError> {
        self.inner.lock().unwrap().file.sync_data()?;
        Ok(())
    }

    #[must_use]
    pub fn reader(self: &Arc<Self>) -> StagingReader {
        StagingReader {
            staging: Arc::clone(self),
            pos: 0,
        }
    }
}

pub struct StagingReader {
    staging: Arc<StagingFile>,
    pos: u64,
}

impl AsyncRead for StagingReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let pos = self.pos;
        let dst = buf.initialize_unfilled();
        match self.staging.read_at(pos, dst) {
            Ok(n) => {
                buf.advance(n);
                self.pos += n as u64;
                Poll::Ready(Ok(()))
            }
            Err(e) => Poll::Ready(Err(io::Error::other(e.to_string()))),
        }
    }
}
