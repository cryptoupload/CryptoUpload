// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg};
use std::path::Path;
use std::sync::Arc;

use cu_format::policy::ProgramAccess;
use cu_mount::sentry::{Access, Sentry, SentryRules};
use cu_mount::write::{HookFuture, MountWriteHooks};

use crate::error::CoreError;
use crate::vault::Vault;

pub use cu_mount::ForcedUnmount;
pub use cu_mount::platform::MountOptions;
pub use cu_mount::sentry::{AskFuture, SentryEvents, Tripwire};
pub use cu_mount::write::WriteOptions;

pub struct MountHandle(
    cu_mount::platform::MountHandle,
    Option<Arc<cu_sync::registry::Registrar>>,
);

impl MountHandle {
    #[must_use]
    pub fn flusher(&self) -> MountFlush {
        MountFlush(self.0.ops(), self.1.clone())
    }

    pub fn unmount(self) -> Result<(), CoreError> {
        self.0.unmount().map_err(|e| CoreError::Mount(e.msg()))
    }

    pub fn unmount_forced(self) -> Result<ForcedUnmount, CoreError> {
        self.0
            .unmount_forced()
            .map_err(|e| CoreError::Mount(e.msg()))
    }
}

#[derive(Clone)]
pub struct MountFlush(
    Arc<cu_mount::VfsOps>,
    Option<Arc<cu_sync::registry::Registrar>>,
);

impl MountFlush {
    #[must_use]
    pub fn pending_files(&self) -> usize {
        self.0.pending_files()
    }

    pub async fn flush(&self) -> usize {
        let failed = self.0.commit_all_dirty().await;
        if let Some(registry) = &self.1 {
            let _ = registry.flush().await;
        }
        failed
    }
}

struct VaultTrash(Arc<Vault>);

impl MountWriteHooks for VaultTrash {
    fn trash<'a>(&'a self, vault_path: &'a str) -> HookFuture<'a> {
        Box::pin(async move {
            match self.0.trash(vault_path).await {
                Ok(()) | Err(CoreError::NotFound) => Ok(()),
                Err(e) => Err(e.to_string()),
            }
        })
    }
}

impl Vault {
    pub async fn mount(
        &self,
        mountpoint: &Path,
        options: MountOptions,
    ) -> Result<MountHandle, CoreError> {
        if options.write.is_some() && self.effective_policy().read_only_drive {
            return Err(CoreError::PolicyForbids(Msg::new("POLICY_DRIVE_READ_ONLY")));
        }
        self.mount_unchecked(mountpoint, options, None).await
    }

    pub async fn mount_guarded(
        &self,
        mountpoint: &Path,
        options: MountOptions,
        write_confirmed: bool,
        events: Arc<dyn SentryEvents>,
    ) -> Result<MountHandle, CoreError> {
        if !write_confirmed && options.write.is_some() && self.effective_policy().read_only_drive {
            return Err(CoreError::PolicyForbids(Msg::new("POLICY_DRIVE_READ_ONLY")));
        }
        self.mount_unchecked(mountpoint, options, Some(events))
            .await
    }

    pub async fn mount_write_confirmed(
        &self,
        mountpoint: &Path,
        options: MountOptions,
    ) -> Result<MountHandle, CoreError> {
        self.mount_unchecked(mountpoint, options, None).await
    }

    fn sentry_rules(&self) -> SentryRules {
        let policy = self.effective_policy();
        SentryRules {
            access: match policy.program_access {
                ProgramAccess::Off => Access::Off,
                ProgramAccess::Allowlist => Access::Allowlist,
                ProgramAccess::Ask => Access::Ask,
            },
            allowed: policy.allowed_programs,
            decoys: policy.decoys,
            mass_read_threshold: policy.mass_read_threshold,
            exempt: policy.exempt_programs,
        }
    }

    async fn mount_unchecked(
        &self,
        mountpoint: &Path,
        mut options: MountOptions,
        events: Option<Arc<dyn SentryEvents>>,
    ) -> Result<MountHandle, CoreError> {
        let rules = self.sentry_rules();
        if rules.access != Access::Off && !cfg!(target_os = "linux") {
            return Err(CoreError::PolicyForbids(Msg::new(
                "POLICY_DRIVE_PROGRAMS_UNSUPPORTED",
            )));
        }
        let sentry = rules
            .is_active()
            .then(|| Arc::new(Sentry::new(rules, events)));
        if let Some(write) = options.write.as_mut()
            && write.hooks.is_none()
        {
            write.hooks = Some(Arc::new(VaultTrash(Arc::new(self.shadow()))));
        }
        let handle = cu_mount::platform::mount_guarded(
            self.fs_arc(),
            self.config().clone(),
            mountpoint,
            options,
            sentry,
        )
        .await
        .map_err(|e| CoreError::Mount(e.msg()))?;
        Ok(MountHandle(handle, self.registrar()))
    }
}
