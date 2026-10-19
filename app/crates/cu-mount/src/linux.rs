// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};
use std::ffi::OsStr;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use cu_format::config::VaultConfig;
use cu_vfs::{NodeKind, VaultFs};
use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, INodeNo, MountOption,
    ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, Request, SessionACL,
};
use fuser::{InitFlags, KernelConfig};
use tokio::runtime::Handle;

use crate::ForcedUnmount;
use crate::inode::ROOT_INO;
use crate::ops::{Attr, OpError, VfsOps};
use crate::sentry::{DECOY_COUNT, DECOY_SIZE, Sentry};
use crate::write::WriteOptions;

const OP_TIMEOUT: Duration = Duration::from_secs(30);

const POSITIVE_ATTR_TTL: Duration = Duration::from_secs(5);
const NEGATIVE_ATTR_TTL: Duration = Duration::from_secs(1);

const GENERATION: fuser::Generation = fuser::Generation(0);

const UNRESOLVED_INO: u64 = u64::MAX;

enum CallError {
    Op(OpError),
    TimedOut,
    Panicked,
}

fn errno_for_op(err: &OpError) -> Errno {
    match err {
        OpError::NotFound => Errno::ENOENT,
        OpError::NotADirectory => Errno::ENOTDIR,
        OpError::IsADirectory => Errno::EISDIR,
        OpError::UnknownHandle => Errno::EBADF,
        OpError::Vfs(_) | OpError::Io(_) => Errno::EIO,
        OpError::Exists => Errno::EEXIST,
        OpError::NotEmpty => Errno::ENOTEMPTY,
        OpError::ReadOnly => Errno::EROFS,
        OpError::Reserved => Errno::EACCES,
    }
}

fn errno_for_call(err: &CallError) -> Errno {
    match err {
        CallError::Op(op) => errno_for_op(op),
        CallError::TimedOut | CallError::Panicked => Errno::EIO,
    }
}

async fn with_timeout_and_panic_guard<T, F>(fut: F) -> Result<T, CallError>
where
    F: Future<Output = Result<T, OpError>> + Send + 'static,
    T: Send + 'static,
{
    let isolated = tokio::spawn(fut);
    match tokio::time::timeout(OP_TIMEOUT, isolated).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(op_err))) => Err(CallError::Op(op_err)),
        Ok(Err(_join_panicked)) => Err(CallError::Panicked),
        Err(_elapsed) => Err(CallError::TimedOut),
    }
}

fn dir_attr(ino: u64, uid: u32, gid: u32, writable: bool) -> FileAttr {
    let perm = if writable { 0o700 } else { 0o500 };
    base_attr(ino, 0, FileType::Directory, perm, 2, uid, gid)
}

fn file_attr(ino: u64, size: u64, uid: u32, gid: u32, writable: bool) -> FileAttr {
    let perm = if writable { 0o600 } else { 0o400 };
    base_attr(ino, size, FileType::RegularFile, perm, 1, uid, gid)
}

fn attr_for(ino: u64, attr: Attr, uid: u32, gid: u32, writable: bool) -> FileAttr {
    match attr.kind {
        NodeKind::Dir => dir_attr(ino, uid, gid, writable),
        NodeKind::File => file_attr(ino, attr.size, uid, gid, writable),
    }
}

#[allow(clippy::too_many_arguments)]
fn base_attr(
    ino: u64,
    size: u64,
    kind: FileType,
    perm: u16,
    nlink: u32,
    uid: u32,
    gid: u32,
) -> FileAttr {
    FileAttr {
        ino: fuser::INodeNo(ino),
        size,
        blocks: size.div_ceil(512),
        atime: SystemTime::UNIX_EPOCH,
        mtime: SystemTime::UNIX_EPOCH,
        ctime: SystemTime::UNIX_EPOCH,
        crtime: SystemTime::UNIX_EPOCH,
        kind,
        perm,
        nlink,
        uid,
        gid,
        rdev: 0,
        blksize: 32_740,
        flags: 0,
    }
}

fn negative_attr() -> FileAttr {
    base_attr(0, 0, FileType::RegularFile, 0, 0, 0, 0)
}

struct VfsFs {
    ops: Arc<VfsOps>,
    rt: Handle,
    sentry: Option<Arc<Sentry>>,
    owner: (u32, u32),
}

fn process_owner() -> std::io::Result<(u32, u32)> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata("/proc/self")?;
    Ok((meta.uid(), meta.gid()))
}

fn decoy_ino(index: usize) -> u64 {
    u64::MAX - 1 - u64::try_from(index).unwrap_or(0)
}

fn is_decoy_ino(ino: u64) -> bool {
    let top = u64::MAX - 1;
    ino <= top && top - ino < u64::try_from(DECOY_COUNT).unwrap_or(0)
}

fn decoy_named(sentry: Option<&Arc<Sentry>>, parent: u64, name: &str) -> Option<u64> {
    if parent != ROOT_INO {
        return None;
    }
    sentry?.decoy_position(name).map(decoy_ino)
}

async fn admitted(sentry: Option<&Arc<Sentry>>, pid: u32) -> bool {
    match sentry {
        Some(sentry) => sentry.admits(pid).await,
        None => true,
    }
}

impl Filesystem for VfsFs {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> std::io::Result<()> {
        let _ = config.add_capabilities(InitFlags::FUSE_DIRECT_IO_ALLOW_MMAP);
        Ok(())
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(name) = name.to_str() else {
            reply.error(Errno::ENOENT);
            return;
        };
        let ops = Arc::clone(&self.ops);
        let name = name.to_string();
        let parent = parent.0;
        let (uid, gid) = self.owner;
        let writable = self.ops.is_writable();
        let decoy = decoy_named(self.sentry.as_ref(), parent, &name);
        self.rt.spawn(async move {
            match with_timeout_and_panic_guard(async move { ops.lookup(parent, &name).await }).await
            {
                Ok((ino, attr)) => reply.entry(
                    &POSITIVE_ATTR_TTL,
                    &attr_for(ino, attr, uid, gid, writable),
                    GENERATION,
                ),
                Err(CallError::Op(OpError::NotFound)) => match decoy {
                    Some(ino) => reply.entry(
                        &POSITIVE_ATTR_TTL,
                        &file_attr(ino, DECOY_SIZE, uid, gid, false),
                        GENERATION,
                    ),
                    None => reply.entry(&NEGATIVE_ATTR_TTL, &negative_attr(), GENERATION),
                },
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        let ops = Arc::clone(&self.ops);
        let ino = ino.0;
        let (uid, gid) = self.owner;
        let writable = self.ops.is_writable();
        if self.sentry.is_some() && is_decoy_ino(ino) {
            reply.attr(
                &POSITIVE_ATTR_TTL,
                &file_attr(ino, DECOY_SIZE, uid, gid, false),
            );
            return;
        }
        self.rt.spawn(async move {
            let cached = fh.and_then(|fh| ops.getattr_for_handle(fh.0).ok());
            let result = match cached {
                Some(attr) => Ok(attr),
                None => with_timeout_and_panic_guard(async move { ops.getattr(ino).await }).await,
            };
            match result {
                Ok(attr) => {
                    reply.attr(&POSITIVE_ATTR_TTL, &attr_for(ino, attr, uid, gid, writable));
                }
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn open(&self, req: &Request, ino: INodeNo, flags: fuser::OpenFlags, reply: ReplyOpen) {
        let ops = Arc::clone(&self.ops);
        let ino = ino.0;
        let wants_write = flags.acc_mode() != fuser::OpenAccMode::O_RDONLY;
        let truncate = flags.0 & O_TRUNC != 0;
        let sentry = self.sentry.clone();
        let pid = req.pid();
        if let Some(sentry) = sentry.as_ref().filter(|_| is_decoy_ino(ino)) {
            sentry.decoy_touched(pid);
            reply.error(Errno::EACCES);
            return;
        }
        self.rt.spawn(async move {
            if !admitted(sentry.as_ref(), pid).await {
                reply.error(Errno::EACCES);
                return;
            }
            if let Some(sentry) = &sentry {
                sentry.file_opened(pid, ino, std::time::Instant::now());
            }
            let opened = with_timeout_and_panic_guard(async move {
                if wants_write {
                    ops.open_write(ino, truncate).await
                } else {
                    ops.open(ino).await
                }
            })
            .await;
            match opened {
                Ok(fh) => reply.opened(FileHandle(fh), FopenFlags::FOPEN_DIRECT_IO),
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: ReplyData,
    ) {
        let ops = Arc::clone(&self.ops);
        let fh = fh.0;
        self.rt.spawn(async move {
            match with_timeout_and_panic_guard(async move { ops.read(fh, offset, size).await })
                .await
            {
                Ok(data) => reply.data(&data),
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn create(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        _flags: i32,
        reply: fuser::ReplyCreate,
    ) {
        let Some(name) = name.to_str() else {
            reply.error(Errno::EINVAL);
            return;
        };
        let ops = Arc::clone(&self.ops);
        let name = name.to_string();
        let parent = parent.0;
        let (uid, gid) = self.owner;
        let sentry = self.sentry.clone();
        let pid = req.pid();
        self.rt.spawn(async move {
            if !admitted(sentry.as_ref(), pid).await {
                reply.error(Errno::EACCES);
                return;
            }
            match with_timeout_and_panic_guard(async move { ops.create(parent, &name).await }).await
            {
                Ok((ino, attr, fh)) => reply.created(
                    &POSITIVE_ATTR_TTL,
                    &attr_for(ino, attr, uid, gid, true),
                    GENERATION,
                    FileHandle(fh),
                    FopenFlags::FOPEN_DIRECT_IO,
                ),
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: fuser::WriteFlags,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        reply: fuser::ReplyWrite,
    ) {
        if self.ops.write_needs_loading(fh.0) {
            let ops = Arc::clone(&self.ops);
            let (fh, data) = (fh.0, data.to_vec());
            self.rt.spawn(async move {
                match ops.write_loading(fh, offset, &data).await {
                    Ok(n) => reply.written(u32::try_from(n).unwrap_or(u32::MAX)),
                    Err(err) => reply.error(errno_for_op(&err)),
                }
            });
            return;
        }
        match self.ops.write(fh.0, offset, data) {
            Ok(n) => reply.written(u32::try_from(n).unwrap_or(u32::MAX)),
            Err(err) => reply.error(errno_for_op(&err)),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock_owner: fuser::LockOwner,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        match self.ops.fsync(fh.0) {
            Ok(()) => reply.ok(),
            Err(err) => reply.error(errno_for_op(&err)),
        }
    }

    fn setattr(
        &self,
        req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<fuser::TimeOrNow>,
        _mtime: Option<fuser::TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let ops = Arc::clone(&self.ops);
        let ino = ino.0;
        let (uid, gid) = self.owner;
        let writable = self.ops.is_writable();
        let sentry = self.sentry.clone();
        let pid = req.pid();
        if sentry.is_some() && is_decoy_ino(ino) {
            reply.error(Errno::EACCES);
            return;
        }
        self.rt.spawn(async move {
            if size.is_some() && !admitted(sentry.as_ref(), pid).await {
                reply.error(Errno::EACCES);
                return;
            }
            let result = with_timeout_and_panic_guard(async move {
                if let Some(size) = size {
                    ops.set_size(ino, size).await?;
                }
                ops.getattr(ino).await
            })
            .await;
            match result {
                Ok(attr) => {
                    reply.attr(&POSITIVE_ATTR_TTL, &attr_for(ino, attr, uid, gid, writable));
                }
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn mkdir(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        let Some(name) = name.to_str() else {
            reply.error(Errno::EINVAL);
            return;
        };
        let ops = Arc::clone(&self.ops);
        let name = name.to_string();
        let parent = parent.0;
        let (uid, gid) = self.owner;
        let sentry = self.sentry.clone();
        let pid = req.pid();
        self.rt.spawn(async move {
            if !admitted(sentry.as_ref(), pid).await {
                reply.error(Errno::EACCES);
                return;
            }
            match with_timeout_and_panic_guard(async move { ops.mkdir(parent, &name).await }).await
            {
                Ok((ino, attr)) => reply.entry(
                    &POSITIVE_ATTR_TTL,
                    &attr_for(ino, attr, uid, gid, true),
                    GENERATION,
                ),
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn unlink(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let Some(name) = name.to_str() else {
            reply.error(Errno::ENOENT);
            return;
        };
        let ops = Arc::clone(&self.ops);
        let name = name.to_string();
        let parent = parent.0;
        let sentry = self.sentry.clone();
        let pid = req.pid();
        let decoy = decoy_named(sentry.as_ref(), parent, &name).is_some();
        self.rt.spawn(async move {
            if decoy && matches!(ops.lookup(parent, &name).await, Err(OpError::NotFound)) {
                if let Some(sentry) = &sentry {
                    sentry.decoy_touched(pid);
                }
                reply.error(Errno::EACCES);
                return;
            }
            if !admitted(sentry.as_ref(), pid).await {
                reply.error(Errno::EACCES);
                return;
            }
            match with_timeout_and_panic_guard(async move { ops.unlink(parent, &name).await }).await
            {
                Ok(()) => reply.ok(),
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn rmdir(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let Some(name) = name.to_str() else {
            reply.error(Errno::ENOENT);
            return;
        };
        let ops = Arc::clone(&self.ops);
        let name = name.to_string();
        let parent = parent.0;
        let sentry = self.sentry.clone();
        let pid = req.pid();
        self.rt.spawn(async move {
            if !admitted(sentry.as_ref(), pid).await {
                reply.error(Errno::EACCES);
                return;
            }
            match with_timeout_and_panic_guard(async move { ops.rmdir(parent, &name).await }).await
            {
                Ok(()) => reply.ok(),
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn rename(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: fuser::RenameFlags,
        reply: ReplyEmpty,
    ) {
        if flags.contains(fuser::RenameFlags::RENAME_EXCHANGE) {
            reply.error(Errno::EINVAL);
            return;
        }
        let (Some(name), Some(newname)) = (name.to_str(), newname.to_str()) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let ops = Arc::clone(&self.ops);
        let (name, newname) = (name.to_string(), newname.to_string());
        let (parent, newparent) = (parent.0, newparent.0);
        let no_replace = flags.contains(fuser::RenameFlags::RENAME_NOREPLACE);
        let sentry = self.sentry.clone();
        let pid = req.pid();
        let decoy = decoy_named(sentry.as_ref(), parent, &name).is_some();
        self.rt.spawn(async move {
            if decoy && matches!(ops.lookup(parent, &name).await, Err(OpError::NotFound)) {
                if let Some(sentry) = &sentry {
                    sentry.decoy_touched(pid);
                }
                reply.error(Errno::EACCES);
                return;
            }
            if !admitted(sentry.as_ref(), pid).await {
                reply.error(Errno::EACCES);
                return;
            }
            match with_timeout_and_panic_guard(async move {
                ops.rename(parent, &name, newparent, &newname, no_replace)
                    .await
            })
            .await
            {
                Ok(()) => reply.ok(),
                Err(err) => reply.error(errno_for_call(&err)),
            }
        });
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: fuser::OpenFlags,
        _lock_owner: Option<fuser::LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let ops = Arc::clone(&self.ops);
        let fh = fh.0;
        self.rt.spawn(async move {
            let _ = ops.release(fh).await;
            reply.ok();
        });
    }

    fn opendir(&self, req: &Request, _ino: INodeNo, _flags: fuser::OpenFlags, reply: ReplyOpen) {
        let Some(sentry) = self.sentry.clone() else {
            reply.opened(FileHandle(self.ops.opendir()), FopenFlags::empty());
            return;
        };
        let ops = Arc::clone(&self.ops);
        let pid = req.pid();
        self.rt.spawn(async move {
            if sentry.admits(pid).await {
                reply.opened(FileHandle(ops.opendir()), FopenFlags::empty());
            } else {
                reply.error(Errno::EACCES);
            }
        });
    }

    fn releasedir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: fuser::OpenFlags,
        reply: ReplyEmpty,
    ) {
        self.ops.releasedir(fh.0);
        reply.ok();
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let ops = Arc::clone(&self.ops);
        let this_ino = ino.0;
        let fh = fh.0;
        let parent_ino = ops.parent_ino(this_ino).unwrap_or(this_ino);
        let sentry = self.sentry.clone();
        self.rt.spawn(async move {
            let result = with_timeout_and_panic_guard(async move {
                ops.dir_snapshot(fh, this_ino, offset == 0).await
            })
            .await;
            let entries = match result {
                Ok(entries) => entries,
                Err(err) => {
                    reply.error(errno_for_call(&err));
                    return;
                }
            };

            let dots = [
                (INodeNo(this_ino), FileType::Directory, ".".to_string()),
                (INodeNo(parent_ino), FileType::Directory, "..".to_string()),
            ];
            let named = entries.iter().map(|e| {
                let kind = match e.kind {
                    NodeKind::Dir => FileType::Directory,
                    NodeKind::File => FileType::RegularFile,
                };
                (INodeNo(UNRESOLVED_INO), kind, e.name.clone())
            });
            let decoy_names = sentry.as_ref().map_or(&[][..], |s| s.decoy_names());
            let decoys: Vec<_> = decoy_names
                .iter()
                .enumerate()
                .filter(|(_, name)| {
                    decoy_named(sentry.as_ref(), this_ino, name).is_some()
                        && !entries.iter().any(|e| e.name == **name)
                })
                .map(|(index, name)| {
                    (
                        INodeNo(decoy_ino(index)),
                        FileType::RegularFile,
                        name.clone(),
                    )
                })
                .collect();

            for (index, (ino, kind, name)) in dots
                .into_iter()
                .chain(named)
                .chain(decoys)
                .enumerate()
                .skip(usize::try_from(offset).unwrap_or(usize::MAX))
            {
                let next_offset = u64::try_from(index + 1).unwrap_or(u64::MAX);
                if reply.add(ino, next_offset, kind, &name) {
                    break;
                }
            }
            reply.ok();
        });
    }
}

#[derive(Debug)]
pub enum MountError {
    Io(std::io::Error),
    Vfs(OpError),
}

impl Coded for MountError {
    fn msg(&self) -> Msg {
        match self {
            Self::Io(e) => Msg::new("IO").arg(e),
            Self::Vfs(e) => e.msg(),
        }
    }
}

display_as_msg!(MountError);

impl std::error::Error for MountError {}

impl From<OpError> for MountError {
    fn from(err: OpError) -> Self {
        Self::Vfs(err)
    }
}

const O_TRUNC: i32 = 0o1000;

#[derive(Debug, Clone)]
pub struct MountOptions {
    pub cache_budget_bytes: usize,
    pub allow_other: bool,
    pub write: Option<WriteOptions>,
}

pub struct MountHandle {
    session: fuser::BackgroundSession,
    ops: Arc<VfsOps>,
    mountpoint: std::path::PathBuf,
}

impl MountHandle {
    #[must_use]
    pub fn ops(&self) -> Arc<VfsOps> {
        Arc::clone(&self.ops)
    }

    pub fn unmount(self) -> Result<(), MountError> {
        self.session.umount_and_join().map_err(MountError::Io)
    }

    pub fn unmount_forced(self) -> Result<ForcedUnmount, MountError> {
        if abort_connection(&self.mountpoint).is_ok() {
            detach_mountpoint(&self.mountpoint);
            self.session.umount_and_join().map_err(MountError::Io)?;
            return Ok(ForcedUnmount::Severed);
        }
        let session = self.session;
        std::thread::spawn(move || {
            let _ = session.umount_and_join();
        });
        Ok(ForcedUnmount::Detached)
    }
}

fn abort_connection(mountpoint: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let dev = std::fs::metadata(mountpoint)?.dev();
    let control = format!("/sys/fs/fuse/connections/{}/abort", dev_minor(dev));
    std::fs::write(control, b"1")
}

fn detach_mountpoint(mountpoint: &Path) {
    for program in ["fusermount3", "fusermount"] {
        let done = std::process::Command::new(program)
            .args(["-u", "-z", "-q", "--"])
            .arg(mountpoint)
            .stdin(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if done {
            return;
        }
    }
}

fn dev_minor(dev: u64) -> u64 {
    (dev & 0xff) | ((dev >> 12) & 0xffff_ff00)
}

pub async fn mount(
    fs: Arc<VaultFs>,
    config: VaultConfig,
    mountpoint: &Path,
    options: MountOptions,
) -> Result<MountHandle, MountError> {
    mount_guarded(fs, config, mountpoint, options, None).await
}

pub async fn mount_guarded(
    fs: Arc<VaultFs>,
    config: VaultConfig,
    mountpoint: &Path,
    options: MountOptions,
    sentry: Option<Arc<Sentry>>,
) -> Result<MountHandle, MountError> {
    let mut vfs_ops =
        VfsOps::with_cache_budget_bytes(fs, config, options.cache_budget_bytes).await?;
    let writable = options.write.is_some();
    if let Some(write) = options.write {
        vfs_ops
            .enable_writes(write.staging_dir, write.hooks)
            .map_err(MountError::Io)?;
        vfs_ops.recover_pending().await;
    }
    let ops = Arc::new(vfs_ops);
    let rt = Handle::current();
    let adapter = VfsFs {
        ops: Arc::clone(&ops),
        rt,
        sentry,
        owner: process_owner().map_err(MountError::Io)?,
    };

    let mut cfg = Config::default();
    cfg.mount_options = vec![
        MountOption::FSName("cryptoupload".to_string()),
        MountOption::Subtype("cryptoupload".to_string()),
    ];
    if !writable {
        cfg.mount_options.push(MountOption::RO);
    }
    if options.allow_other {
        cfg.mount_options.push(MountOption::DefaultPermissions);
        cfg.mount_options
            .push(MountOption::CUSTOM("allow_other".to_string()));
        cfg.acl = SessionACL::All;
    }

    let session = fuser::spawn_mount(adapter, mountpoint, &cfg).map_err(MountError::Io)?;
    Ok(MountHandle {
        session,
        ops,
        mountpoint: mountpoint.to_path_buf(),
    })
}
