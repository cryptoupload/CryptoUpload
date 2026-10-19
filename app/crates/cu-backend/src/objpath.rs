// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;

use crate::error::BackendError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjPath(String);

impl ObjPath {
    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    pub fn new(path: impl Into<String>) -> Result<Self, BackendError> {
        let path = path.into();
        if path.is_empty() {
            return Ok(Self::root());
        }
        if path.starts_with('/') || path.ends_with('/') {
            return Err(BackendError::InvalidPath(
                Msg::new("BACKEND_PATH_SLASH").arg(path),
            ));
        }
        for segment in path.split('/') {
            if segment.is_empty() {
                return Err(BackendError::InvalidPath(
                    Msg::new("BACKEND_PATH_EMPTY_SEGMENT").arg(path),
                ));
            }
            if segment == ".." || segment == "." {
                return Err(BackendError::InvalidPath(
                    Msg::new("BACKEND_PATH_NAVIGATION").arg(path),
                ));
            }
        }
        Ok(Self(path))
    }

    pub fn join(&self, segment: &str) -> Result<Self, BackendError> {
        if segment.is_empty() || segment.contains('/') || segment == ".." || segment == "." {
            return Err(BackendError::InvalidPath(
                Msg::new("BACKEND_PATH_SEGMENT").arg(segment),
            ));
        }
        if self.0.is_empty() {
            Ok(Self(segment.to_string()))
        } else {
            Ok(Self(format!("{}/{segment}", self.0)))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}
