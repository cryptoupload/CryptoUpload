// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::BackendError;
use cu_format::FormatError;
use cu_msg::{Coded, Msg, display_as_msg};
use cu_sync::hooks::HookOutcome;

#[derive(Debug)]
pub enum CoreError {
    NotFound,
    AlreadyExists,
    NotAVault,
    InvalidName(String),
    GuardRejected(Msg),
    RecoveryPhraseAlreadyPresent,
    NoRecoveryPhrase,
    Format(FormatError),
    Backend(BackendError),
    Io(std::io::Error),
    InvalidProxy(Msg),
    Index(cu_index::IndexError),
    RandomUnavailable,
    InvalidLocalName(String),
    VersionNotFound,
    ContentUnavailable,
    ReservedName(String),
    DataDirUnavailable(Msg),
    IntegrityFailure(Msg),
    Mount(Msg),
    PlatformKeyUnavailable,
    PlatformKeychain(Msg),
    Fido2(Msg),
    RegistryDisabled,
    PreHookFailed(HookOutcome),
    AnomalyDetected {
        reason: Vec<Msg>,
        uploads_affected: usize,
        deletes_affected: usize,
        renames_affected: usize,
    },
    Jobs(Msg),
    Cancelled,
    Share(Msg),
    DirectoryChanged(String),
    ConfigChangedElsewhere,
    VaultIdentityChanged,
    WeakPassword {
        min_chars: usize,
    },
    ConfigRolledBack {
        served: u64,
        remembered: u64,
    },
    RecoveryShares(Msg),
    TwoFactorRequired,
    PolicyReauthRequired,
    PolicyForbids(Msg),
}

impl CoreError {
    #[must_use]
    pub fn is_decrypt_failure(&self) -> bool {
        matches!(self, Self::Format(FormatError::DecryptFailed))
    }
}

impl Coded for CoreError {
    #[allow(clippy::too_many_lines)]
    fn msg(&self) -> Msg {
        match self {
            Self::NotFound => Msg::new("PATH_NOT_FOUND"),
            Self::AlreadyExists => Msg::new("DESTINATION_EXISTS"),
            Self::NotAVault => Msg::new("NOT_A_VAULT"),
            Self::InvalidName(name) => Msg::new("INVALID_NAME").arg(name),
            Self::GuardRejected(msg)
            | Self::Jobs(msg)
            | Self::Share(msg)
            | Self::Mount(msg)
            | Self::InvalidProxy(msg)
            | Self::DataDirUnavailable(msg)
            | Self::RecoveryShares(msg)
            | Self::Fido2(msg)
            | Self::PlatformKeychain(msg)
            | Self::IntegrityFailure(msg)
            | Self::PolicyForbids(msg) => msg.clone(),
            Self::RecoveryPhraseAlreadyPresent => Msg::new("CORE_RECOVERY_PHRASE_PRESENT"),
            Self::NoRecoveryPhrase => Msg::new("NO_RECOVERY_PHRASE"),
            Self::Format(e) => e.msg(),
            Self::Backend(e) => e.msg(),
            Self::Io(e) => Msg::new("IO").arg(e),
            Self::Index(e) => e.msg(),
            Self::RandomUnavailable => Msg::new("RANDOM_UNAVAILABLE"),
            Self::InvalidLocalName(name) => Msg::new("SYNC_INVALID_LOCAL_NAME").arg(name),
            Self::VersionNotFound => Msg::new("VERSION_NOT_FOUND"),
            Self::ContentUnavailable => Msg::new("CONTENT_UNAVAILABLE"),
            Self::ReservedName(path) => Msg::new("SYNC_RESERVED_NAME").arg(path),
            Self::PlatformKeyUnavailable => Msg::new("PLATFORM_KEY_UNAVAILABLE"),
            Self::RegistryDisabled => Msg::new("REGISTRY_DISABLED"),
            Self::PreHookFailed(outcome) => {
                cu_sync::SyncError::PreHookFailed(outcome.clone()).msg()
            }
            Self::AnomalyDetected { reason, .. } => reason
                .first()
                .cloned()
                .unwrap_or_else(|| Msg::new("SYNC_ANOMALY")),
            Self::Cancelled => Msg::new("SYNC_CANCELLED"),
            Self::DirectoryChanged(path) => Msg::new("SYNC_DIRECTORY_CHANGED").arg(path),
            Self::ConfigChangedElsewhere => Msg::new("CONFIG_CHANGED_ELSEWHERE"),
            Self::VaultIdentityChanged => Msg::new("VAULT_IDENTITY_CHANGED"),
            Self::WeakPassword { min_chars } => Msg::new("WEAK_PASSWORD").arg(min_chars),
            Self::ConfigRolledBack { served, remembered } => {
                Msg::new("CONFIG_ROLLED_BACK").arg(served).arg(remembered)
            }
            Self::TwoFactorRequired => Msg::new("TWO_FACTOR_REQUIRED"),
            Self::PolicyReauthRequired => Msg::new("POLICY_REAUTH_REQUIRED"),
        }
    }
}

display_as_msg!(CoreError);

impl std::error::Error for CoreError {}

impl From<FormatError> for CoreError {
    fn from(err: FormatError) -> Self {
        match err {
            FormatError::NoRecoveryRecipient => Self::NoRecoveryPhrase,
            FormatError::TwoFactorRequired => Self::TwoFactorRequired,
            other => Self::Format(other),
        }
    }
}

impl From<BackendError> for CoreError {
    fn from(err: BackendError) -> Self {
        match err {
            BackendError::NotFound => Self::NotFound,
            BackendError::AlreadyExists => Self::AlreadyExists,
            other => Self::Backend(other),
        }
    }
}

impl From<cu_vfs::VfsError> for CoreError {
    fn from(err: cu_vfs::VfsError) -> Self {
        match err {
            cu_vfs::VfsError::NotFound => Self::NotFound,
            cu_vfs::VfsError::AlreadyExists => Self::AlreadyExists,
            cu_vfs::VfsError::InvalidName(name) => Self::InvalidName(name),
            cu_vfs::VfsError::Format(e) => Self::Format(e),
            cu_vfs::VfsError::Backend(e) => Self::Backend(e),
            cu_vfs::VfsError::Registry(msg) => Self::IntegrityFailure(msg),
        }
    }
}

impl From<std::io::Error> for CoreError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<cu_index::IndexError> for CoreError {
    fn from(err: cu_index::IndexError) -> Self {
        Self::Index(err)
    }
}

impl From<cu_jobs::JobsError> for CoreError {
    fn from(err: cu_jobs::JobsError) -> Self {
        Self::Jobs(err.msg())
    }
}

impl From<cu_sync::SyncError> for CoreError {
    fn from(err: cu_sync::SyncError) -> Self {
        match err {
            cu_sync::SyncError::Io(e) => Self::Io(e),
            cu_sync::SyncError::Index(e) => Self::Index(e),
            cu_sync::SyncError::Vfs(e) => e.into(),
            cu_sync::SyncError::RandomUnavailable => Self::RandomUnavailable,
            cu_sync::SyncError::InvalidLocalName(name) => Self::InvalidLocalName(name),
            cu_sync::SyncError::ReservedName(path) => Self::ReservedName(path),
            cu_sync::SyncError::VersionNotFound => Self::VersionNotFound,
            cu_sync::SyncError::ContentUnavailable => Self::ContentUnavailable,
            cu_sync::SyncError::IntegrityFailure(reason) => Self::IntegrityFailure(reason),
            cu_sync::SyncError::PreHookFailed(reason) => Self::PreHookFailed(reason),
            cu_sync::SyncError::AnomalyDetected {
                reason,
                uploads_affected,
                deletes_affected,
                renames_affected,
            } => Self::AnomalyDetected {
                reason,
                uploads_affected,
                deletes_affected,
                renames_affected,
            },
            cu_sync::SyncError::Cancelled => Self::Cancelled,
            cu_sync::SyncError::DirectoryChanged(path) => Self::DirectoryChanged(path),
        }
    }
}

impl CoreError {
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
