// SPDX-License-Identifier: GPL-3.0-or-later

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    AppFolder,
    DriveRoot,
}

impl RootKind {
    #[must_use]
    pub(crate) const fn root_addr(self) -> &'static str {
        match self {
            Self::AppFolder => "/me/drive/special/approot",
            Self::DriveRoot => "/me/drive/root",
        }
    }

    #[must_use]
    pub(crate) const fn file_scope(self) -> &'static str {
        match self {
            Self::AppFolder => "Files.ReadWrite.AppFolder",
            Self::DriveRoot => "Files.ReadWrite",
        }
    }
}

#[derive(Clone)]
pub struct OneDriveConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub tenant: String,
    pub refresh_token: String,
    pub root_kind: RootKind,
}

impl OneDriveConfig {
    pub const DEFAULT_TENANT: &'static str = "common";
}
