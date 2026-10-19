// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};
use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cu_format::config::VaultConfig;
use cu_vfs::{NodeKind, VaultFs};
use tokio::runtime::Handle;
use widestring::U16CStr;
use windows::Win32::Foundation::{
    STATUS_ACCESS_DENIED, STATUS_BUFFER_OVERFLOW, STATUS_DIRECTORY_NOT_EMPTY,
    STATUS_FILE_IS_A_DIRECTORY, STATUS_INVALID_HANDLE, STATUS_IO_DEVICE_ERROR, STATUS_IO_TIMEOUT,
    STATUS_MEDIA_WRITE_PROTECTED, STATUS_NOT_A_DIRECTORY, STATUS_OBJECT_NAME_COLLISION,
    STATUS_OBJECT_NAME_NOT_FOUND,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_READONLY,
};
use winfsp::filesystem::{
    AsyncFileSystemContext, DirInfo, DirMarker, FileInfo, FileSecurity, FileSystemContext,
    OpenFileInfo, VolumeInfo, WideNameInfo,
};
use winfsp::host::{FileSystemHost, FineGuard, VolumeParams};
use winfsp_sys::{FILE_ACCESS_RIGHTS, FILE_FLAGS_AND_ATTRIBUTES};

use crate::ForcedUnmount;
use crate::inode::{Ino, ROOT_INO};
use crate::ops::{Attr, Fh, OpError, VfsOps};
use crate::write::WriteOptions;

const FILE_WRITE_DATA: u32 = 0x0002;
const FILE_APPEND_DATA: u32 = 0x0004;
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const CLEANUP_DELETE: u32 = 0x01;

const OP_TIMEOUT: Duration = Duration::from_secs(30);

const WINDOWS_EPOCH_OFFSET_100NS: u64 = 116_444_736_000_000_000;

fn op_error_to_status(err: &OpError) -> winfsp::FspError {
    match err {
        OpError::NotFound => STATUS_OBJECT_NAME_NOT_FOUND.into(),
        OpError::NotADirectory => STATUS_NOT_A_DIRECTORY.into(),
        OpError::IsADirectory => STATUS_FILE_IS_A_DIRECTORY.into(),
        OpError::UnknownHandle => STATUS_INVALID_HANDLE.into(),
        OpError::Vfs(_) | OpError::Io(_) => STATUS_IO_DEVICE_ERROR.into(),
        OpError::ReadOnly | OpError::Reserved => STATUS_ACCESS_DENIED.into(),
        OpError::Exists => STATUS_OBJECT_NAME_COLLISION.into(),
        OpError::NotEmpty => STATUS_DIRECTORY_NOT_EMPTY.into(),
    }
}

fn timed_out() -> winfsp::FspError {
    STATUS_IO_TIMEOUT.into()
}

#[derive(Clone, Copy)]
enum FileCtx {
    Dir { ino: Ino, dh: Fh },
    File { ino: Ino, fh: Fh },
}

impl FileCtx {
    fn ino(self) -> Ino {
        match self {
            FileCtx::Dir { ino, .. } | FileCtx::File { ino, .. } => ino,
        }
    }
}

type DirListing = Arc<Vec<(String, Attr)>>;

struct VfsFs {
    ops: Arc<VfsOps>,
    rt: Handle,
    dir_listings: Mutex<HashMap<Fh, DirListing>>,
    security: Vec<u8>,
}

impl VfsFs {
    fn write_security(&self, buffer: Option<&mut [c_void]>) -> winfsp::Result<u64> {
        if let Some(buffer) = buffer {
            if !copy_into_winfsp_buffer(buffer, &self.security) {
                return Err(STATUS_BUFFER_OVERFLOW.into());
            }
        }
        Ok(self.security.len() as u64)
    }

    fn resolve_path(&self, path: &U16CStr) -> Result<(Ino, Attr), OpError> {
        let path = path.to_string_lossy();
        self.rt.block_on(async {
            let mut ino = ROOT_INO;
            let mut attr = self.ops.getattr(ROOT_INO).await?;
            for component in path.split('\\').filter(|c| !c.is_empty()) {
                let (next_ino, next_attr) = self.ops.lookup(ino, component).await?;
                ino = next_ino;
                attr = next_attr;
            }
            Ok((ino, attr))
        })
    }

    fn resolve_parent(&self, path: &U16CStr) -> Result<(Ino, String), OpError> {
        let path = path.to_string_lossy();
        let mut components: Vec<&str> = path.split('\\').filter(|c| !c.is_empty()).collect();
        let Some(name) = components.pop() else {
            return Err(OpError::NotFound);
        };
        self.rt.block_on(async {
            let mut ino = ROOT_INO;
            for component in components {
                let (next, _) = self.ops.lookup(ino, component).await?;
                ino = next;
            }
            Ok((ino, name.to_string()))
        })
    }

    fn call_blocking<T>(&self, fut: impl Future<Output = Result<T, OpError>>) -> winfsp::Result<T> {
        self.rt.block_on(async {
            match tokio::time::timeout(OP_TIMEOUT, fut).await {
                Ok(result) => result.map_err(|e| op_error_to_status(&e)),
                Err(_) => Err(timed_out()),
            }
        })
    }

    async fn read_directory_entries(&self, ino: Ino) -> Result<Vec<(String, Attr)>, OpError> {
        let entries = self.ops.readdir_with_attrs(ino).await?;
        let mut out = Vec::with_capacity(entries.len() + 2);
        out.push((".".to_string(), self.ops.getattr(ino).await?));
        let parent = self.ops.parent_ino(ino).unwrap_or(ino);
        out.push(("..".to_string(), self.ops.getattr(parent).await?));
        out.extend(entries);
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    async fn listing(&self, ino: Ino, dh: Fh, fresh: bool) -> Result<DirListing, OpError> {
        if !fresh {
            if let Some(listing) = self.dir_listings.lock().unwrap().get(&dh) {
                return Ok(Arc::clone(listing));
            }
        }
        let listing = Arc::new(self.read_directory_entries(ino).await?);
        self.dir_listings
            .lock()
            .unwrap()
            .insert(dh, Arc::clone(&listing));
        Ok(listing)
    }
}

fn file_attributes(kind: NodeKind, writable: bool) -> u32 {
    match (kind, writable) {
        (NodeKind::Dir, _) => FILE_ATTRIBUTE_DIRECTORY.0,
        (NodeKind::File, false) => FILE_ATTRIBUTE_READONLY.0,
        (NodeKind::File, true) => FILE_ATTRIBUTE_NORMAL.0,
    }
}

fn fill_file_info(fi: &mut FileInfo, attr: Attr, writable: bool) {
    *fi = FileInfo::default();
    fi.file_attributes = file_attributes(attr.kind, writable);
    fi.file_size = attr.size;
    fi.allocation_size = attr.size;
    fi.creation_time = WINDOWS_EPOCH_OFFSET_100NS;
    fi.last_access_time = WINDOWS_EPOCH_OFFSET_100NS;
    fi.last_write_time = WINDOWS_EPOCH_OFFSET_100NS;
    fi.change_time = WINDOWS_EPOCH_OFFSET_100NS;
}

impl FileSystemContext for VfsFs {
    type FileContext = FileCtx;

    fn get_security_by_name(
        &self,
        file_name: &U16CStr,
        security_descriptor: Option<&mut [c_void]>,
        _reparse_point_resolver: impl FnOnce(&U16CStr) -> Option<FileSecurity>,
    ) -> winfsp::Result<FileSecurity> {
        let (_ino, attr) = self
            .resolve_path(file_name)
            .map_err(|e| op_error_to_status(&e))?;
        Ok(FileSecurity {
            reparse: false,
            sz_security_descriptor: self.write_security(security_descriptor)?,
            attributes: file_attributes(attr.kind, self.ops.is_writable()),
        })
    }

    fn get_security(
        &self,
        _context: &Self::FileContext,
        security_descriptor: Option<&mut [c_void]>,
    ) -> winfsp::Result<u64> {
        self.write_security(security_descriptor)
    }

    fn open(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        granted_access: FILE_ACCESS_RIGHTS,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let (ino, attr) = self
            .resolve_path(file_name)
            .map_err(|e| op_error_to_status(&e))?;
        fill_file_info(file_info.as_mut(), attr, self.ops.is_writable());
        if attr.kind == NodeKind::Dir {
            Ok(FileCtx::Dir {
                ino,
                dh: self.ops.opendir(),
            })
        } else {
            let wants_write = granted_access & (FILE_WRITE_DATA | FILE_APPEND_DATA) != 0;
            let fh = if wants_write && self.ops.is_writable() {
                self.call_blocking(self.ops.open_write(ino, false))?
            } else {
                self.call_blocking(self.ops.open(ino))?
            };
            Ok(FileCtx::File { ino, fh })
        }
    }

    fn close(&self, context: Self::FileContext) {
        match context {
            FileCtx::File { fh, .. } => {
                let _ = self.rt.block_on(self.ops.release(fh));
            }
            FileCtx::Dir { dh, .. } => {
                self.dir_listings.lock().unwrap().remove(&dh);
                self.ops.releasedir(dh);
            }
        }
    }

    fn get_file_info(
        &self,
        context: &Self::FileContext,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        let attr = if let FileCtx::File { fh, .. } = context {
            self.ops
                .getattr_for_handle(*fh)
                .map_err(|e| op_error_to_status(&e))?
        } else {
            self.call_blocking(self.ops.getattr(context.ino()))?
        };
        fill_file_info(file_info, attr, self.ops.is_writable());
        Ok(())
    }

    fn get_volume_info(&self, out_volume_info: &mut VolumeInfo) -> winfsp::Result<()> {
        const FAKE_TOTAL_SIZE: u64 = 1024 * 1024 * 1024 * 1024;
        out_volume_info.total_size = FAKE_TOTAL_SIZE;
        out_volume_info.free_size = if self.ops.is_writable() {
            FAKE_TOTAL_SIZE
        } else {
            0
        };
        out_volume_info.set_volume_label("CryptoUpload");
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        file_name: &U16CStr,
        create_options: u32,
        _granted_access: FILE_ACCESS_RIGHTS,
        file_attributes: FILE_FLAGS_AND_ATTRIBUTES,
        _security_descriptor: Option<&[c_void]>,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        _extra_buffer_is_reparse_point: bool,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        if !self.ops.is_writable() {
            return Err(STATUS_MEDIA_WRITE_PROTECTED.into());
        }
        let (parent, name) = self
            .resolve_parent(file_name)
            .map_err(|e| op_error_to_status(&e))?;
        let is_dir = create_options & FILE_DIRECTORY_FILE != 0
            || file_attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
        if is_dir {
            let (ino, attr) = self.call_blocking(self.ops.mkdir(parent, &name))?;
            fill_file_info(file_info.as_mut(), attr, true);
            Ok(FileCtx::Dir {
                ino,
                dh: self.ops.opendir(),
            })
        } else {
            let (ino, attr, fh) = self.call_blocking(self.ops.create(parent, &name))?;
            fill_file_info(file_info.as_mut(), attr, true);
            Ok(FileCtx::File { ino, fh })
        }
    }

    fn overwrite(
        &self,
        context: &Self::FileContext,
        _file_attributes: FILE_FLAGS_AND_ATTRIBUTES,
        _replace_file_attributes: bool,
        _allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        let FileCtx::File { ino, fh } = *context else {
            return Err(STATUS_FILE_IS_A_DIRECTORY.into());
        };
        self.call_blocking(self.ops.set_size(ino, 0))?;
        let attr = self
            .ops
            .getattr_for_handle(fh)
            .map_err(|e| op_error_to_status(&e))?;
        fill_file_info(file_info, attr, true);
        Ok(())
    }

    fn set_file_size(
        &self,
        context: &Self::FileContext,
        new_size: u64,
        set_allocation_size: bool,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        let FileCtx::File { ino, fh } = *context else {
            return Err(STATUS_FILE_IS_A_DIRECTORY.into());
        };
        let current = self
            .ops
            .getattr_for_handle(fh)
            .map_err(|e| op_error_to_status(&e))?
            .size;
        if !set_allocation_size || new_size < current {
            self.call_blocking(self.ops.set_size(ino, new_size))?;
        }
        let attr = self
            .ops
            .getattr_for_handle(fh)
            .map_err(|e| op_error_to_status(&e))?;
        fill_file_info(file_info, attr, true);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn set_basic_info(
        &self,
        context: &Self::FileContext,
        _file_attributes: u32,
        _creation_time: u64,
        _last_access_time: u64,
        _last_write_time: u64,
        _last_change_time: u64,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        self.get_file_info(context, file_info)
    }

    fn flush(
        &self,
        context: Option<&Self::FileContext>,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        if let Some(ctx) = context {
            if let FileCtx::File { fh, .. } = *ctx {
                self.ops.fsync(fh).map_err(|e| op_error_to_status(&e))?;
            }
            self.get_file_info(ctx, file_info)?;
        }
        Ok(())
    }

    fn set_delete(
        &self,
        context: &Self::FileContext,
        _file_name: &U16CStr,
        delete_file: bool,
    ) -> winfsp::Result<()> {
        if delete_file {
            if let FileCtx::Dir { ino, .. } = *context {
                let entries = self.call_blocking(self.ops.readdir(ino))?;
                if !entries.is_empty() {
                    return Err(STATUS_DIRECTORY_NOT_EMPTY.into());
                }
            }
        }
        Ok(())
    }

    fn cleanup(&self, context: &Self::FileContext, file_name: Option<&U16CStr>, flags: u32) {
        if flags & CLEANUP_DELETE == 0 {
            return;
        }
        let Some(file_name) = file_name else {
            return;
        };
        let Ok((parent, name)) = self.resolve_parent(file_name) else {
            return;
        };
        let _ = self.call_blocking(async {
            match context {
                FileCtx::File { .. } => self.ops.unlink(parent, &name).await,
                FileCtx::Dir { .. } => self.ops.rmdir(parent, &name).await,
            }
        });
    }

    fn rename(
        &self,
        _context: &Self::FileContext,
        file_name: &U16CStr,
        new_file_name: &U16CStr,
        replace_if_exists: bool,
    ) -> winfsp::Result<()> {
        let (old_parent, old_name) = self
            .resolve_parent(file_name)
            .map_err(|e| op_error_to_status(&e))?;
        let (new_parent, new_name) = self
            .resolve_parent(new_file_name)
            .map_err(|e| op_error_to_status(&e))?;
        self.call_blocking(self.ops.rename(
            old_parent,
            &old_name,
            new_parent,
            &new_name,
            !replace_if_exists,
        ))
    }
}

impl AsyncFileSystemContext for VfsFs {
    async fn read_async(
        &self,
        context: &Self::FileContext,
        buffer: &mut [u8],
        offset: u64,
    ) -> winfsp::Result<u32> {
        let FileCtx::File { fh, .. } = *context else {
            return Err(STATUS_FILE_IS_A_DIRECTORY.into());
        };
        let len = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        match tokio::time::timeout(OP_TIMEOUT, self.ops.read(fh, offset, len)).await {
            Ok(Ok(data)) => {
                buffer[..data.len()].copy_from_slice(&data);
                Ok(u32::try_from(data.len()).unwrap_or(len))
            }
            Ok(Err(e)) => Err(op_error_to_status(&e)),
            Err(_) => Err(timed_out()),
        }
    }

    async fn write_async(
        &self,
        context: &Self::FileContext,
        buffer: &[u8],
        offset: u64,
        write_to_eof: bool,
        constrained_io: bool,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<u32> {
        let FileCtx::File { fh, .. } = *context else {
            return Err(STATUS_FILE_IS_A_DIRECTORY.into());
        };
        let current = self
            .ops
            .getattr_for_handle(fh)
            .map_err(|e| op_error_to_status(&e))?
            .size;
        let offset = if write_to_eof { current } else { offset };
        let data = if constrained_io {
            if offset >= current {
                fill_file_info(
                    file_info,
                    Attr {
                        kind: NodeKind::File,
                        size: current,
                    },
                    true,
                );
                return Ok(0);
            }
            let room = usize::try_from(current - offset).unwrap_or(usize::MAX);
            buffer.get(..room.min(buffer.len())).unwrap_or(buffer)
        } else {
            buffer
        };
        let written = if self.ops.write_needs_loading(fh) {
            self.ops.write_loading(fh, offset, data).await
        } else {
            self.ops.write(fh, offset, data)
        }
        .map_err(|e| op_error_to_status(&e))?;
        let attr = self
            .ops
            .getattr_for_handle(fh)
            .map_err(|e| op_error_to_status(&e))?;
        fill_file_info(file_info, attr, true);
        Ok(u32::try_from(written).unwrap_or(u32::MAX))
    }

    async fn read_directory_async(
        &self,
        context: &Self::FileContext,
        _pattern: Option<&U16CStr>,
        marker: DirMarker<'_>,
        buffer: &mut [u8],
    ) -> winfsp::Result<u32> {
        let FileCtx::Dir { ino, dh } = *context else {
            return Err(STATUS_NOT_A_DIRECTORY.into());
        };
        let after = marker.inner_as_cstr().map(U16CStr::to_string_lossy);
        let entries =
            match tokio::time::timeout(OP_TIMEOUT, self.listing(ino, dh, after.is_none())).await {
                Ok(Ok(entries)) => entries,
                Ok(Err(e)) => return Err(op_error_to_status(&e)),
                Err(_) => return Err(timed_out()),
            };

        let mut cursor = 0u32;
        for (name, attr) in entries.iter() {
            if let Some(after) = &after {
                if name.as_str() <= after.as_str() {
                    continue;
                }
            }
            let mut dir_info = DirInfo::<255>::new();
            let wide: Vec<u16> = name.encode_utf16().collect();
            if dir_info.set_name_raw(wide.as_slice()).is_err() {
                continue;
            }
            fill_file_info(dir_info.file_info_mut(), *attr, self.ops.is_writable());
            if !dir_info.append_to_buffer(buffer, &mut cursor) {
                return Ok(cursor);
            }
        }
        DirInfo::<255>::finalize_buffer(buffer, &mut cursor);
        Ok(cursor)
    }

    fn spawn_task(&self, future: impl Future<Output = ()> + Send + 'static) {
        self.rt.spawn(future);
    }
}

fn owner_only_sddl(user_sid: &str) -> String {
    format!("O:{user_sid}G:{user_sid}D:P(A;;FA;;;{user_sid})(A;;FA;;;SY)")
}

#[allow(unsafe_code)]
fn owner_only_security_descriptor() -> std::io::Result<Vec<u8>> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::{PCWSTR, PWSTR};

    let os = |e: windows::core::Error| std::io::Error::from_raw_os_error(e.code().0);

    // SAFETY: valid pointers, aligned buffer; Windows allocations copied, then freed once.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token).map_err(os)?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &raw mut needed);
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        let queried = GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &raw mut needed,
        );
        let _ = CloseHandle(token);
        queried.map_err(os)?;
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();

        let mut string_sid = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &raw mut string_sid).map_err(os)?;
        let sid = string_sid.to_string();
        let _ = LocalFree(Some(HLOCAL(string_sid.0.cast())));
        let sid = sid.map_err(|_| std::io::Error::other("SID"))?;

        let sddl: Vec<u16> = owner_only_sddl(&sid)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut size = 0u32;
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &raw mut descriptor,
            Some(&raw mut size),
        )
        .map_err(os)?;
        let bytes = std::slice::from_raw_parts(descriptor.0.cast::<u8>(), size as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        Ok(bytes)
    }
}

#[allow(unsafe_code)]
fn copy_into_winfsp_buffer(buffer: &mut [c_void], bytes: &[u8]) -> bool {
    if buffer.len() < bytes.len() {
        return false;
    }
    // SAFETY: `bytes` fits in `buffer` and they do not overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            buffer.as_mut_ptr().cast::<u8>(),
            bytes.len(),
        );
    }
    true
}

#[derive(Debug)]
pub enum MountError {
    Vfs(OpError),
    Fsp(winfsp::FspError),
    Io(std::io::Error),
}

impl Coded for MountError {
    fn msg(&self) -> Msg {
        match self {
            MountError::Vfs(e) => e.msg(),
            MountError::Fsp(e) => Msg::new("MOUNT_WINFSP")
                .arg(e)
                .arg(format!("0x{:08X}", e.to_ntstatus())),
            MountError::Io(e) => Msg::new("IO").arg(e),
        }
    }
}

display_as_msg!(MountError);

impl std::error::Error for MountError {}

impl From<OpError> for MountError {
    fn from(e: OpError) -> Self {
        MountError::Vfs(e)
    }
}

impl From<winfsp::FspError> for MountError {
    fn from(e: winfsp::FspError) -> Self {
        MountError::Fsp(e)
    }
}

#[derive(Debug, Clone)]
pub struct MountOptions {
    pub cache_budget_bytes: usize,
    pub allow_other: bool,
    pub write: Option<WriteOptions>,
}

pub struct MountHandle {
    _host: FileSystemHost<VfsFs>,
    runtime: Option<tokio::runtime::Runtime>,
    ops: Arc<VfsOps>,
}

impl MountHandle {
    #[must_use]
    pub fn ops(&self) -> Arc<VfsOps> {
        Arc::clone(&self.ops)
    }

    pub fn unmount(self) -> Result<(), MountError> {
        Ok(())
    }
}

impl MountHandle {
    pub fn unmount_forced(self) -> Result<ForcedUnmount, MountError> {
        self.unmount()?;
        Ok(ForcedUnmount::Severed)
    }
}

impl Drop for MountHandle {
    fn drop(&mut self) {
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

pub async fn mount_guarded(
    fs: Arc<VaultFs>,
    config: VaultConfig,
    mountpoint: &Path,
    options: MountOptions,
    _sentry: Option<Arc<crate::sentry::Sentry>>,
) -> Result<MountHandle, MountError> {
    mount(fs, config, mountpoint, options).await
}

pub async fn mount(
    fs: Arc<VaultFs>,
    config: VaultConfig,
    mountpoint: &Path,
    options: MountOptions,
) -> Result<MountHandle, MountError> {
    winfsp::winfsp_init()?;

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

    let dedicated_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("cu-mount-winfsp")
        .enable_all()
        .build()
        .map_err(MountError::Io)?;
    let rt = dedicated_rt.handle().clone();
    let vfsfs = VfsFs {
        ops: Arc::clone(&ops),
        rt,
        dir_listings: Mutex::new(HashMap::new()),
        security: owner_only_security_descriptor().map_err(MountError::Io)?,
    };

    let mut params = VolumeParams::new();
    params
        .case_sensitive_search(true)
        .case_preserved_names(true)
        .unicode_on_disk(true)
        .persistent_acls(true)
        .read_only_volume(!writable)
        .flush_and_purge_on_cleanup(true)
        .file_info_timeout(5_000)
        .sector_size(4096)
        .sectors_per_allocation_unit(1)
        .filesystem_name("CryptoUpload")
        .volume_serial_number(0x4355_5031);

    let mut host: FileSystemHost<VfsFs, FineGuard> =
        FileSystemHost::new_async(params, vfsfs).map_err(winfsp::FspError::from)?;

    if mountpoint.symlink_metadata().is_ok() {
        std::fs::remove_dir(mountpoint).map_err(MountError::Io)?;
    }
    host.mount(mountpoint).map_err(winfsp::FspError::from)?;
    host.start().map_err(winfsp::FspError::from)?;
    Ok(MountHandle {
        _host: host,
        runtime: Some(dedicated_rt),
        ops,
    })
}
