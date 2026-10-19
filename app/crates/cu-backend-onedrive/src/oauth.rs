// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::BackendError;
use cu_oauth::{ConsentRequest, Endpoints, OAuthError, RedirectPort};

use crate::config::RootKind;

fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn endpoints_for_tenant(tenant: &str) -> Endpoints {
    Endpoints {
        auth_url: leak_str(format!(
            "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
        )),
        token_url: leak_str(format!(
            "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"
        )),
    }
}

fn map_err(e: &OAuthError) -> BackendError {
    BackendError::Other(format!("onedrive: {e}"))
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
    client_secret: Option<&str>,
    tenant: &str,
    root_kind: RootKind,
) -> Result<PendingConsent, BackendError> {
    let endpoints = endpoints_for_tenant(tenant);
    let request = ConsentRequest {
        client_id: client_id.to_string(),
        client_secret: client_secret.map(str::to_string),
        scopes: vec![
            "offline_access".to_string(),
            root_kind.file_scope().to_string(),
        ],
        extra_auth_params: Vec::new(),
        redirect_port: RedirectPort::Ephemeral,
        redirect_host: "127.0.0.1",
        label: "OneDrive",
    };
    let inner = cu_oauth::begin_authorization(&endpoints, request)
        .await
        .map_err(|e| map_err(&e))?;
    Ok(PendingConsent {
        auth_url: inner.auth_url.clone(),
        inner,
    })
}

pub async fn refresh_access_token(
    client_id: &str,
    client_secret: Option<&str>,
    tenant: &str,
    refresh_token: &str,
) -> Result<String, BackendError> {
    let refreshed =
        refresh_access_token_with_expiry(client_id, client_secret, tenant, refresh_token).await?;
    Ok(refreshed.access_token)
}

pub(crate) async fn refresh_access_token_with_expiry(
    client_id: &str,
    client_secret: Option<&str>,
    tenant: &str,
    refresh_token: &str,
) -> Result<cu_oauth::RefreshedToken, BackendError> {
    refresh_with_endpoints(
        &endpoints_for_tenant(tenant),
        client_id,
        client_secret,
        refresh_token,
    )
    .await
}

pub(crate) async fn refresh_with_endpoints(
    endpoints: &Endpoints,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<cu_oauth::RefreshedToken, BackendError> {
    cu_oauth::refresh_access_token(endpoints, client_id, client_secret, refresh_token)
        .await
        .map_err(|e| map_err(&e))
}

pub(crate) fn tenant_endpoints(tenant: &str) -> Endpoints {
    endpoints_for_tenant(tenant)
}
