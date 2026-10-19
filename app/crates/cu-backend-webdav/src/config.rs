// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::BackendError;

#[derive(Clone)]
pub struct WebDavConfig {
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub root_path: String,
    pub allow_insecure_http: bool,
}

impl WebDavConfig {
    pub(crate) fn validate(&self) -> Result<(), BackendError> {
        if self.base_url.starts_with("https://") {
            return Ok(());
        }
        if self.base_url.starts_with("http://") {
            if self.allow_insecure_http {
                return Ok(());
            }
            return Err(BackendError::Coded(cu_backend::Msg::new(
                "WEBDAV_HTTP_INSECURE",
            )));
        }
        Err(BackendError::Coded(cu_backend::Msg::new(
            "WEBDAV_URL_SCHEME",
        )))
    }
}
