// SPDX-License-Identifier: GPL-3.0-or-later

use crate::hooks::HookOutcome;
use cu_format::FormatError;
use cu_index::IndexError;
use cu_msg::{Coded, Msg, display_as_msg};
use cu_vfs::VfsError;

#[derive(Debug)]
pub enum SyncError {
    Io(std::io::Error),
    Index(IndexError),
    Vfs(VfsError),
    RandomUnavailable,
    InvalidLocalName(String),
    ReservedName(String),
    VersionNotFound,
    ContentUnavailable,
    IntegrityFailure(Msg),
    PreHookFailed(HookOutcome),
    AnomalyDetected {
        reason: Vec<Msg>,
        uploads_affected: usize,
        deletes_affected: usize,
        renames_affected: usize,
    },
    Cancelled,
    DirectoryChanged(String),
}

impl Coded for SyncError {
    fn msg(&self) -> Msg {
        match self {
            Self::Io(e) => Msg::new("IO").arg(e),
            Self::Index(e) => e.msg(),
            Self::Vfs(e) => e.msg(),
            Self::RandomUnavailable => Msg::new("RANDOM_UNAVAILABLE"),
            Self::InvalidLocalName(name) => Msg::new("SYNC_INVALID_LOCAL_NAME").arg(name),
            Self::ReservedName(path) => Msg::new("SYNC_RESERVED_NAME").arg(path),
            Self::VersionNotFound => Msg::new("VERSION_NOT_FOUND"),
            Self::ContentUnavailable => Msg::new("CONTENT_UNAVAILABLE"),
            Self::IntegrityFailure(why) => why.clone(),
            Self::PreHookFailed(outcome) => match outcome {
                HookOutcome::Failed {
                    exit_code: Some(code),
                    stderr_tail,
                } => Msg::new("SYNC_PRE_HOOK_EXIT").arg(code).arg(stderr_tail),
                HookOutcome::Failed {
                    exit_code: None,
                    stderr_tail,
                } => Msg::new("SYNC_PRE_HOOK_SIGNAL").arg(stderr_tail),
                HookOutcome::TimedOut => Msg::new("SYNC_PRE_HOOK_TIMEOUT"),
                HookOutcome::SpawnFailed(reason) => Msg::new("SYNC_PRE_HOOK_SPAWN").arg(reason),
                HookOutcome::Ok => Msg::new("SYNC_PRE_HOOK_EXIT").arg("0").arg(""),
            },
            Self::AnomalyDetected { reason, .. } => reason
                .first()
                .cloned()
                .unwrap_or_else(|| Msg::new("SYNC_ANOMALY")),
            Self::Cancelled => Msg::new("SYNC_CANCELLED"),
            Self::DirectoryChanged(path) => Msg::new("SYNC_DIRECTORY_CHANGED").arg(path),
        }
    }
}

display_as_msg!(SyncError);

impl std::error::Error for SyncError {}

impl From<std::io::Error> for SyncError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<IndexError> for SyncError {
    fn from(err: IndexError) -> Self {
        Self::Index(err)
    }
}

impl From<VfsError> for SyncError {
    fn from(err: VfsError) -> Self {
        Self::Vfs(err)
    }
}

impl From<cu_crypto::CryptoError> for SyncError {
    fn from(_: cu_crypto::CryptoError) -> Self {
        Self::RandomUnavailable
    }
}

impl From<FormatError> for SyncError {
    fn from(err: FormatError) -> Self {
        Self::IntegrityFailure(err.msg())
    }
}
