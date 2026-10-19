// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use cu_backend::{
    BackendError, Capabilities, EntryKind, ListEntry, ObjPath, ObjStat, StorageBackend,
    StreamReader, StreamWriter,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub struct FsBackend {
    root: PathBuf,
}

impl FsBackend {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn resolve(&self, path: &ObjPath) -> PathBuf {
        if path.is_root() {
            self.root.clone()
        } else {
            self.root.join(path.as_str())
        }
    }
}

#[async_trait]
impl StorageBackend for FsBackend {
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
        Ok(tokio::fs::try_exists(self.resolve(path)).await?)
    }

    async fn get(&self, path: &ObjPath) -> Result<Vec<u8>, BackendError> {
        Ok(tokio::fs::read(self.resolve(path)).await?)
    }

    async fn get_range(
        &self,
        path: &ObjPath,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        let mut file = tokio::fs::File::open(self.resolve(path)).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let want = usize::try_from(len).unwrap_or(usize::MAX);
        let mut buf = vec![0u8; want];
        let mut filled = 0;
        while filled < want {
            let n = file.read(&mut buf[filled..]).await?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        buf.truncate(filled);
        Ok(buf)
    }

    async fn stat(&self, path: &ObjPath) -> Result<ObjStat, BackendError> {
        let meta = tokio::fs::metadata(self.resolve(path)).await?;
        Ok(ObjStat { size: meta.len() })
    }

    async fn put(&self, path: &ObjPath, data: &[u8]) -> Result<(), BackendError> {
        let dest = self.resolve(path);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = tmp_path(&dest);
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(data).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, &dest).await?;
        fsync_parent_dir(&dest).await?;
        Ok(())
    }

    async fn open_read(&self, path: &ObjPath) -> Result<Box<dyn StreamReader>, BackendError> {
        let file = tokio::fs::File::open(self.resolve(path)).await?;
        Ok(Box::new(FsStreamReader {
            file,
            ahead: Vec::new(),
            pos: 0,
        }))
    }

    async fn open_write(&self, path: &ObjPath) -> Result<Box<dyn StreamWriter>, BackendError> {
        let dest = self.resolve(path);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = tmp_path(&dest);
        let file = tokio::fs::File::create(&tmp).await?;
        Ok(Box::new(FsStreamWriter {
            file: Some(tokio::io::BufWriter::with_capacity(WRITE_BUFFER, file)),
            tmp,
            dest,
            exclusive: false,
            published: false,
        }))
    }

    async fn open_write_exclusive(
        &self,
        path: &ObjPath,
    ) -> Result<Box<dyn StreamWriter>, BackendError> {
        let dest = self.resolve(path);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = tmp_path(&dest);
        let file = tokio::fs::File::create(&tmp).await?;
        Ok(Box::new(FsStreamWriter {
            file: Some(tokio::io::BufWriter::with_capacity(WRITE_BUFFER, file)),
            tmp,
            dest,
            exclusive: true,
            published: false,
        }))
    }

    async fn delete(&self, path: &ObjPath) -> Result<(), BackendError> {
        match tokio::fs::remove_file(self.resolve(path)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    async fn list(&self, dir: &ObjPath) -> Result<Vec<ListEntry>, BackendError> {
        let mut read_dir = tokio::fs::read_dir(self.resolve(dir)).await?;
        let mut entries = Vec::new();
        while let Some(entry) = read_dir.next_entry().await? {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let file_type = entry.file_type().await?;
            let (kind, size, modified) = if file_type.is_dir() {
                (EntryKind::Directory, None, None)
            } else {
                let meta = entry.metadata().await?;
                (
                    EntryKind::File,
                    Some(meta.len()),
                    unix_secs(meta.modified().ok()),
                )
            };
            entries.push(ListEntry {
                name,
                kind,
                size,
                modified,
            });
        }
        Ok(entries)
    }

    async fn create_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        tokio::fs::create_dir_all(self.resolve(dir)).await?;
        Ok(())
    }

    async fn rename(
        &self,
        from: &ObjPath,
        to: &ObjPath,
        _from_kind: EntryKind,
    ) -> Result<(), BackendError> {
        let dest = self.resolve(to);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(self.resolve(from), &dest).await?;
        Ok(())
    }

    async fn remove_dir_all(&self, dir: &ObjPath) -> Result<(), BackendError> {
        match tokio::fs::remove_dir_all(self.resolve(dir)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

const READ_AHEAD: usize = 1024 * 1024;

struct FsStreamReader {
    file: tokio::fs::File,
    ahead: Vec<u8>,
    pos: usize,
}

#[async_trait]
impl StreamReader for FsStreamReader {
    async fn read_chunk(&mut self, max_len: usize) -> Result<Vec<u8>, BackendError> {
        let mut out = Vec::with_capacity(max_len);
        while out.len() < max_len {
            if self.pos == self.ahead.len() {
                self.ahead.resize(READ_AHEAD, 0);
                let mut filled = 0;
                while filled < READ_AHEAD {
                    let n = self.file.read(&mut self.ahead[filled..]).await?;
                    if n == 0 {
                        break;
                    }
                    filled += n;
                }
                self.ahead.truncate(filled);
                self.pos = 0;
                if filled == 0 {
                    break;
                }
            }
            let take = (max_len - out.len()).min(self.ahead.len() - self.pos);
            out.extend_from_slice(&self.ahead[self.pos..self.pos + take]);
            self.pos += take;
        }
        Ok(out)
    }
}

const WRITE_BUFFER: usize = 1024 * 1024;

struct FsStreamWriter {
    file: Option<tokio::io::BufWriter<tokio::fs::File>>,
    tmp: PathBuf,
    dest: PathBuf,
    exclusive: bool,
    published: bool,
}

#[async_trait]
impl StreamWriter for FsStreamWriter {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        self.file
            .as_mut()
            .expect("write_chunk is never called after commit")
            .write_all(data)
            .await?;
        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<(), BackendError> {
        let mut file = self
            .file
            .take()
            .expect("commit consumes self: called at most once");
        file.flush().await?;
        file.get_ref().sync_all().await?;
        drop(file);
        let result = if self.exclusive {
            match tokio::fs::hard_link(&self.tmp, &self.dest).await {
                Ok(()) => {
                    let _ = tokio::fs::remove_file(&self.tmp).await;
                    self.published = true;
                    Ok(())
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(BackendError::AlreadyExists)
                }
                Err(_) => {
                    let outcome = reserve_then_move(&self.tmp, &self.dest).await;
                    self.published = outcome.is_ok();
                    outcome
                }
            }
        } else {
            tokio::fs::rename(&self.tmp, &self.dest).await?;
            self.published = true;
            Ok(())
        };
        if self.published {
            fsync_parent_dir(&self.dest).await?;
        }
        result
    }
}

async fn reserve_then_move(tmp: &Path, dest: &Path) -> Result<(), BackendError> {
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .await
    {
        Ok(_) => match tokio::fs::rename(tmp, dest).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = tokio::fs::remove_file(dest).await;
                Err(e.into())
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(BackendError::AlreadyExists),
        Err(e) => Err(e.into()),
    }
}

impl Drop for FsStreamWriter {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

#[cfg(unix)]
async fn fsync_parent_dir(path: &Path) -> Result<(), BackendError> {
    if let Some(parent) = path.parent() {
        let dir = tokio::fs::File::open(parent).await?;
        dir.sync_all().await?;
    }
    Ok(())
}

#[cfg(not(unix))]
async fn fsync_parent_dir(_path: &Path) -> Result<(), BackendError> {
    Ok(())
}

const TMP_NAME_PREFIX_CHARS: usize = 48;

fn tmp_path(dest: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let file_name: String = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("obj")
        .chars()
        .take(TMP_NAME_PREFIX_CHARS)
        .collect();
    dest.with_file_name(format!(
        ".{file_name}.cryptoupload-tmp.{pid}.{nanos}.{counter}"
    ))
}

fn unix_secs(t: Option<std::time::SystemTime>) -> Option<i64> {
    let d = t?.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_secs()).ok()
}
