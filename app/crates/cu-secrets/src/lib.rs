// SPDX-License-Identifier: GPL-3.0-or-later
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod error;
pub mod file;
pub mod keychain;
pub mod member_identity;

pub(crate) const SALT_LEN: usize = 16;

pub use error::SecretsError;
pub use file::{
    DropboxRemoteConfig, FileFormat, GDriveRemoteConfig, OneDriveRemoteConfig, OneDriveRootKind,
    RemoteConfig, S3ObjectLockMode, S3RemoteConfig, SecretsFile, SftpAuth, SftpRemoteConfig,
    WebDavRemoteConfig,
};
