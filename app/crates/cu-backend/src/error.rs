// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};

#[derive(Debug)]
pub enum BackendError {
    NotFound,
    AlreadyExists,
    Io(std::io::Error),
    InvalidPath(Msg),
    Transient(String),
    Other(String),
    RetentionActive(String),
    Coded(Msg),
}

impl Coded for BackendError {
    fn msg(&self) -> Msg {
        match self {
            Self::NotFound => Msg::new("OBJECT_NOT_FOUND"),
            Self::AlreadyExists => Msg::new("OBJECT_EXISTS"),
            Self::Io(e) => Msg::new("IO").arg(e),
            Self::InvalidPath(msg) | Self::Coded(msg) => msg.clone(),
            Self::Transient(why) => Msg::new("TRANSIENT").arg(why),
            Self::Other(why) => Msg::new("BACKEND_ERROR").arg(why),
            Self::RetentionActive(why) => Msg::new("BACKEND_RETENTION_ACTIVE").arg(why),
        }
    }
}

display_as_msg!(BackendError);

impl std::error::Error for BackendError {}

impl BackendError {
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
}

#[must_use]
pub const fn is_transient_http_status(status: u16) -> bool {
    status == 429 || (status >= 500 && status <= 599)
}

impl From<std::io::Error> for BackendError {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => Self::NotFound,
            _ => Self::Io(err),
        }
    }
}
