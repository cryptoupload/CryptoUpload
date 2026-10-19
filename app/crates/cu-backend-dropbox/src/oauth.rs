// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use cu_backend::BackendError;
use cu_oauth::{ConsentRequest, Endpoints, OAuthError, RedirectPort};

const ENDPOINTS: Endpoints = Endpoints {
    auth_url: "https://www.dropbox.com/oauth2/authorize",
    token_url: "https://api.dropboxapi.com/oauth2/token",
};

const REDIRECT_PORT: u16 = 53_682;

const SCOPES: &[&str] = &[
    "files.metadata.read",
    "files.metadata.write",
    "files.content.read",
    "files.content.write",
];

fn map_err(e: &OAuthError) -> BackendError {
    BackendError::Other(format!("dropbox: {e}"))
}

pub struct PendingConsent {
    pub auth_url: String,
    inner: cu_oauth::PendingConsent,
}

impl PendingConsent {
    pub async fn wait_for_consent(self) -> Result<String, BackendError> {
        self.inner.wait_for_consent().await.map_err(|e| map_err(&e))
    }
}

pub async fn begin_authorization(client_id: &str) -> Result<PendingConsent, BackendError> {
    let request = ConsentRequest {
        client_id: client_id.to_string(),
        client_secret: None,
        scopes: SCOPES.iter().map(|s| (*s).to_string()).collect(),
        extra_auth_params: vec![("token_access_type".to_string(), "offline".to_string())],
        redirect_port: RedirectPort::Fixed(REDIRECT_PORT),
        redirect_host: "127.0.0.1",
        label: "Dropbox",
    };
    let inner = cu_oauth::begin_authorization(&ENDPOINTS, request)
        .await
        .map_err(|e| map_err(&e))?;
    Ok(PendingConsent {
        auth_url: inner.auth_url.clone(),
        inner,
    })
}

pub async fn refresh_access_token(
    client_id: &str,
    refresh_token: &str,
) -> Result<String, BackendError> {
    let (token, _) = refresh_access_token_with_expiry(client_id, refresh_token).await?;
    Ok(token)
}

pub(crate) async fn refresh_access_token_with_expiry(
    client_id: &str,
    refresh_token: &str,
) -> Result<(String, Option<Duration>), BackendError> {
    cu_oauth::refresh_access_token(&ENDPOINTS, client_id, None, refresh_token)
        .await
        .map(|t| (t.access_token, t.expires_in))
        .map_err(|e| map_err(&e))
}
