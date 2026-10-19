// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use subtle::ConstantTimeEq as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone, Copy)]
pub struct Endpoints {
    pub auth_url: &'static str,
    pub token_url: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub enum RedirectPort {
    Ephemeral,
    Fixed(u16),
}

pub struct ConsentRequest {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    pub extra_auth_params: Vec<(String, String)>,
    pub redirect_port: RedirectPort,
    pub redirect_host: &'static str,
    pub label: &'static str,
}

#[derive(Debug)]
pub enum OAuthError {
    Listen(std::io::Error),
    PortInUse(u16),
    Url(String),
    Callback(String),
    Exchange(String),
    ConsentDenied,
    ConsentTimeout,
    NoRefreshToken,
    Http(String),
}

impl Coded for OAuthError {
    fn msg(&self) -> Msg {
        match self {
            Self::Listen(e) => Msg::new("OAUTH_LISTEN").arg(e),
            Self::PortInUse(port) => Msg::new("OAUTH_PORT_IN_USE").arg(port),
            Self::Url(e) => Msg::new("OAUTH_URL").arg(e),
            Self::Callback(e) => Msg::new("OAUTH_CALLBACK").arg(e),
            Self::Exchange(e) => Msg::new("OAUTH_EXCHANGE").arg(e),
            Self::ConsentDenied => Msg::new("OAUTH_CONSENT_DENIED"),
            Self::ConsentTimeout => Msg::new("OAUTH_CONSENT_TIMEOUT"),
            Self::NoRefreshToken => Msg::new("OAUTH_NO_REFRESH_TOKEN"),
            Self::Http(e) => Msg::new("OAUTH_HTTP").arg(e),
        }
    }
}

display_as_msg!(OAuthError);

impl std::error::Error for OAuthError {}

type Client = BasicClient<
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

static PROXY_URL: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

pub fn set_proxy_url(url: Option<String>) {
    *PROXY_URL
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = url;
}

fn http_client() -> Result<reqwest::Client, OAuthError> {
    let proxy_url = PROXY_URL
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let mut builder = reqwest::Client::builder();
    if let Some(url) = proxy_url {
        builder =
            builder.proxy(reqwest::Proxy::all(url).map_err(|e| OAuthError::Http(e.to_string()))?);
    }
    builder
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| OAuthError::Http(e.to_string()))
}

fn build_client(
    endpoints: &Endpoints,
    client_id: &str,
    client_secret: Option<&str>,
    redirect_uri: &str,
) -> Result<Client, OAuthError> {
    let mut client = BasicClient::new(ClientId::new(client_id.to_string()))
        .set_auth_uri(
            AuthUrl::new(endpoints.auth_url.to_string())
                .map_err(|e| OAuthError::Url(e.to_string()))?,
        )
        .set_token_uri(
            TokenUrl::new(endpoints.token_url.to_string())
                .map_err(|e| OAuthError::Url(e.to_string()))?,
        )
        .set_redirect_uri(
            RedirectUrl::new(redirect_uri.to_string())
                .map_err(|e| OAuthError::Url(e.to_string()))?,
        );
    if let Some(secret) = client_secret {
        client = client.set_client_secret(ClientSecret::new(secret.to_string()));
    }
    Ok(client)
}

async fn bind_listener(port: RedirectPort) -> Result<TcpListener, OAuthError> {
    match port {
        RedirectPort::Ephemeral => TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(OAuthError::Listen),
        RedirectPort::Fixed(port) => TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                OAuthError::PortInUse(port)
            } else {
                OAuthError::Listen(e)
            }
        }),
    }
}

pub async fn begin_authorization(
    endpoints: &Endpoints,
    request: ConsentRequest,
) -> Result<PendingConsent, OAuthError> {
    let listener = bind_listener(request.redirect_port).await?;
    let port = listener.local_addr().map_err(OAuthError::Listen)?.port();
    let redirect_uri = format!("http://{}:{port}", request.redirect_host);

    let client = build_client(
        endpoints,
        &request.client_id,
        request.client_secret.as_deref(),
        &redirect_uri,
    )?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let mut auth_request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_challenge);
    for scope in &request.scopes {
        auth_request = auth_request.add_scope(Scope::new(scope.clone()));
    }
    for (key, value) in &request.extra_auth_params {
        auth_request = auth_request.add_extra_param(key.clone(), value.clone());
    }
    let (auth_url, csrf_token) = auth_request.url();

    Ok(PendingConsent {
        auth_url: auth_url.to_string(),
        client_id: request.client_id,
        client_secret: request.client_secret,
        redirect_uri,
        endpoints: *endpoints,
        label: request.label,
        csrf_token,
        pkce_verifier: Arc::new(pkce_verifier),
        listener,
    })
}

pub struct PendingConsent {
    pub auth_url: String,
    client_id: String,
    client_secret: Option<String>,
    redirect_uri: String,
    endpoints: Endpoints,
    label: &'static str,
    csrf_token: CsrfToken,
    pkce_verifier: Arc<PkceCodeVerifier>,
    listener: TcpListener,
}

impl PendingConsent {
    pub async fn wait_for_consent(self) -> Result<String, OAuthError> {
        let code = receive_callback(
            &self.listener,
            self.label,
            self.csrf_token.secret(),
            CALLBACK_TIMEOUT,
        )
        .await?;

        let client = build_client(
            &self.endpoints,
            &self.client_id,
            self.client_secret.as_deref(),
            &self.redirect_uri,
        )?;
        let http = http_client()?;
        let verifier = Arc::try_unwrap(self.pkce_verifier)
            .unwrap_or_else(|arc| PkceCodeVerifier::new(arc.secret().clone()));
        let token = client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(verifier)
            .request_async(&http)
            .await
            .map_err(|e| OAuthError::Exchange(e.to_string()))?;

        token
            .refresh_token()
            .map(|t| t.secret().clone())
            .ok_or(OAuthError::NoRefreshToken)
    }
}

pub struct RefreshedToken {
    pub access_token: String,
    pub expires_in: Option<Duration>,
    pub refresh_token: Option<String>,
}

impl fmt::Debug for RefreshedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshedToken")
            .field("access_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

pub async fn refresh_access_token(
    endpoints: &Endpoints,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<RefreshedToken, OAuthError> {
    let client = build_client(endpoints, client_id, client_secret, "http://127.0.0.1:0")?;
    let http = http_client()?;
    let token = client
        .exchange_refresh_token(&oauth2::RefreshToken::new(refresh_token.to_string()))
        .request_async(&http)
        .await
        .map_err(|e| OAuthError::Exchange(e.to_string()))?;
    Ok(RefreshedToken {
        access_token: token.access_token().secret().clone(),
        expires_in: token.expires_in(),
        refresh_token: token.refresh_token().map(|t| t.secret().clone()),
    })
}

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(15 * 60);

const CONNECTION_READ_TIMEOUT: Duration = Duration::from_secs(10);

async fn read_request_line(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let read = async {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).await.ok()?;
            buf.extend_from_slice(&chunk[..n]);
            if n == 0 || buf.contains(&b'\n') || buf.len() > 64 * 1024 {
                break;
            }
        }
        Some(
            String::from_utf8_lossy(&buf)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string(),
        )
    };
    tokio::time::timeout(CONNECTION_READ_TIMEOUT, read)
        .await
        .ok()
        .flatten()
}

fn parse_redirect(first_line: &str) -> (Option<String>, Option<String>, Option<String>) {
    let path = first_line.split_whitespace().nth(1).unwrap_or_default();
    let query = path.split_once('?').map_or("", |(_, q)| q);
    let (mut code, mut state, mut error) = (None, None, None);
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }
    (code, state, error)
}

static CONSENT_PAGES: std::sync::RwLock<Option<(String, String)>> = std::sync::RwLock::new(None);

pub fn set_consent_pages(granted: String, denied: String) {
    *CONSENT_PAGES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((granted, denied));
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn consent_texts() -> (String, String) {
    CONSENT_PAGES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| {
            (
                "Access granted for {label}. You can close this tab and go back to the app."
                    .to_string(),
                "Access was not granted.".to_string(),
            )
        })
}

fn page(text: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body><p>{}</p></body></html>",
        escape_html(text)
    )
}

fn granted_page(label: &str) -> String {
    page(&consent_texts().0.replace("{label}", label))
}

fn denied_page() -> String {
    page(&consent_texts().1)
}

async fn reply(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn receive_callback(
    listener: &TcpListener,
    label: &str,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, OAuthError> {
    let state_matches = |state: &str| {
        state
            .as_bytes()
            .ct_eq(expected_state.as_bytes())
            .unwrap_u8()
            == 1
    };
    let wait = async {
        loop {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|e| OAuthError::Callback(format!("accept on the local listener: {e}")))?;
            let Some(first_line) = read_request_line(&mut stream).await else {
                continue;
            };
            match parse_redirect(&first_line) {
                (Some(code), Some(state), _) if state_matches(&state) => {
                    let body = granted_page(label);
                    reply(&mut stream, "200 OK", &body).await;
                    return Ok(code);
                }
                (_, Some(state), Some(_)) if state_matches(&state) => {
                    reply(&mut stream, "200 OK", &denied_page()).await;
                    return Err(OAuthError::ConsentDenied);
                }
                (_, Some(_), _) | (_, _, Some(_)) => {
                    reply(&mut stream, "400 Bad Request", "").await;
                }
                _ => reply(&mut stream, "404 Not Found", "").await,
            }
        }
    };
    tokio::time::timeout(timeout, wait)
        .await
        .unwrap_or(Err(OAuthError::ConsentTimeout))
}
