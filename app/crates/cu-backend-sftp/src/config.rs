// SPDX-License-Identifier: GPL-3.0-or-later

#[derive(Clone)]
pub enum SftpAuth {
    Password(String),
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
}

#[derive(Clone)]
pub struct SftpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SftpAuth,
    pub remote_root: String,
    pub expected_host_key_fingerprint: String,
}

pub(crate) const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub(crate) fn client_config() -> std::sync::Arc<russh::client::Config> {
    std::sync::Arc::new(russh::client::Config {
        inactivity_timeout: Some(std::time::Duration::from_secs(300)),
        keepalive_interval: Some(std::time::Duration::from_secs(15)),
        keepalive_max: 3,
        ..russh::client::Config::default()
    })
}

pub(crate) async fn connect<H>(
    config: std::sync::Arc<russh::client::Config>,
    host: &str,
    port: u16,
    handler: H,
) -> Result<russh::client::Handle<H>, cu_backend::BackendError>
where
    H: russh::client::Handler + Send + 'static,
    H::Error: std::fmt::Display,
{
    use cu_backend::BackendError;
    use cu_backend::proxy::ProxyKind;
    let failed = |e: &dyn std::fmt::Display| BackendError::Transient(format!("sftp: connect: {e}"));
    match cu_backend::proxy::global() {
        None => russh::client::connect(config, (host, port), handler)
            .await
            .map_err(|e| failed(&e)),
        Some(proxy) if proxy.kind() == ProxyKind::Socks5 => {
            let stream = crate::socks5::connect(proxy.host(), proxy.port(), host, port)
                .await
                .map_err(|e| BackendError::Transient(format!("sftp: proxy SOCKS5: {e}")))?;
            russh::client::connect_stream(config, stream, handler)
                .await
                .map_err(|e| failed(&e))
        }
        Some(_) => Err(BackendError::Coded(cu_backend::Msg::new("SFTP_PROXY_HTTP"))),
    }
}
