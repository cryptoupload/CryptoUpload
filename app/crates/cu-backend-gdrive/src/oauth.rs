// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::BackendError;
use cu_oauth::{ConsentRequest, Endpoints, OAuthError, RedirectPort};

const ENDPOINTS: Endpoints = Endpoints {
    auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
    token_url: "https://oauth2.googleapis.com/token",
};
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

fn map_err(e: &OAuthError) -> BackendError {
    BackendError::Other(format!("gdrive: {e}"))
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

pub async fn begin_authorization(
    client_id: &str,
    client_secret: &str,
) -> Result<PendingConsent, BackendError> {
    let request = ConsentRequest {
        client_id: client_id.to_string(),
        client_secret: Some(client_secret.to_string()),
        scopes: vec![SCOPE.to_string()],
        extra_auth_params: vec![
            ("access_type".to_string(), "offline".to_string()),
            ("prompt".to_string(), "consent".to_string()),
        ],
        redirect_port: RedirectPort::Ephemeral,
        redirect_host: "127.0.0.1",
        label: "Google Drive",
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
    client_secret: &str,
    refresh_token: &str,
) -> Result<String, BackendError> {
    let refreshed =
        refresh_access_token_with_expiry(client_id, client_secret, refresh_token).await?;
    Ok(refreshed.access_token)
}

pub(crate) async fn refresh_access_token_with_expiry(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<cu_oauth::RefreshedToken, BackendError> {
    refresh_with_endpoints(&ENDPOINTS, client_id, client_secret, refresh_token).await
}

pub(crate) async fn refresh_with_endpoints(
    endpoints: &Endpoints,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<cu_oauth::RefreshedToken, BackendError> {
    cu_oauth::refresh_access_token(endpoints, client_id, Some(client_secret), refresh_token)
        .await
        .map_err(|e| map_err(&e))
}

pub(crate) const fn google_endpoints() -> Endpoints {
    ENDPOINTS
}
