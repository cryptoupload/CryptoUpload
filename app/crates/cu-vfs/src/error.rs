// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::BackendError;
use cu_format::FormatError;
use cu_msg::{Coded, Msg, display_as_msg};

#[derive(Debug)]
pub enum VfsError {
    NotFound,
    AlreadyExists,
    InvalidName(String),
    Format(FormatError),
    Backend(BackendError),
    Registry(Msg),
}

impl Coded for VfsError {
    fn msg(&self) -> Msg {
        match self {
            Self::NotFound => Msg::new("PATH_NOT_FOUND"),
            Self::AlreadyExists => Msg::new("DESTINATION_EXISTS"),
            Self::InvalidName(name) => Msg::new("INVALID_NAME").arg(name),
            Self::Format(e) => e.msg(),
            Self::Backend(e) => e.msg(),
            Self::Registry(msg) => msg.clone(),
        }
    }
}

display_as_msg!(VfsError);

impl std::error::Error for VfsError {}

impl From<FormatError> for VfsError {
    fn from(err: FormatError) -> Self {
        Self::Format(err)
    }
}

impl From<BackendError> for VfsError {
    fn from(err: BackendError) -> Self {
        match err {
            BackendError::NotFound => Self::NotFound,
            BackendError::AlreadyExists => Self::AlreadyExists,
            other => Self::Backend(other),
        }
    }
}
