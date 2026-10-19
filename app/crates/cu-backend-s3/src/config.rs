// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_smithy_types::timeout::TimeoutConfig;

fn timeout_config() -> TimeoutConfig {
    TimeoutConfig::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(60))
        .operation_attempt_timeout(Duration::from_secs(300))
        .build()
}

#[derive(Clone)]
pub struct S3Config {
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
    pub access_key: String,
    pub secret_key: String,
    pub object_lock_mode: Option<ObjectLockMode>,
    pub object_lock_days: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectLockMode {
    Governance,
    Compliance,
}

impl ObjectLockMode {
    pub(crate) fn to_sdk(self) -> aws_sdk_s3::types::ObjectLockMode {
        match self {
            Self::Governance => aws_sdk_s3::types::ObjectLockMode::Governance,
            Self::Compliance => aws_sdk_s3::types::ObjectLockMode::Compliance,
        }
    }
}

pub(crate) fn retain_until_date(days: Option<u16>) -> Option<aws_smithy_types::DateTime> {
    let days = days.filter(|&d| d > 0)?;
    let now_secs = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
    .unwrap_or(0);
    Some(aws_smithy_types::DateTime::from_secs(
        now_secs.saturating_add(i64::from(days).saturating_mul(86_400)),
    ))
}

impl S3Config {
    #[must_use]
    pub fn client(&self) -> Client {
        let credentials = Credentials::new(
            self.access_key.clone(),
            self.secret_key.clone(),
            None,
            None,
            "cu-backend-s3",
        );
        let mut builder = aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(self.region.clone()))
            .credentials_provider(credentials)
            .timeout_config(timeout_config())
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .force_path_style(self.path_style);
        if let Some(endpoint) = &self.endpoint {
            builder = builder.endpoint_url(endpoint.clone());
        }
        if let Some(http_client) = proxied_http_client() {
            builder = builder.http_client(http_client);
        }
        Client::from_conf(builder.build())
    }
}

fn proxy_url_for_s3() -> Option<String> {
    use cu_backend::proxy::{Proxy, ProxyKind};
    let proxy = cu_backend::proxy::global()?;
    Some(match proxy.kind() {
        ProxyKind::Http => proxy.url(),
        ProxyKind::Socks5 => Proxy::unreachable().url(),
    })
}

pub fn check_proxy_supported() -> Result<(), cu_backend::BackendError> {
    match cu_backend::proxy::global() {
        Some(proxy) if proxy.kind() == cu_backend::proxy::ProxyKind::Socks5 => Err(
            cu_backend::BackendError::Coded(cu_backend::Msg::new("S3_PROXY_SOCKS5")),
        ),
        _ => Ok(()),
    }
}

fn proxied_http_client() -> Option<aws_smithy_runtime_api::client::http::SharedHttpClient> {
    use aws_smithy_http_client::{Connector, proxy::ProxyConfig, tls};
    let url = proxy_url_for_s3()?;
    let config = ProxyConfig::all(url.as_str())
        .or_else(|_| ProxyConfig::all(cu_backend::proxy::Proxy::unreachable().url().as_str()))
        .ok()?;
    Some(
        aws_smithy_http_client::Builder::new().build_with_connector_fn(
            move |settings, components| {
                let mut builder = Connector::builder()
                    .proxy_config(config.clone())
                    .tls_provider(tls::Provider::Rustls(
                        tls::rustls_provider::CryptoMode::AwsLc,
                    ));
                builder.set_connector_settings(settings.cloned());
                if let Some(components) = components {
                    builder.set_sleep_impl(components.sleep_impl());
                }
                builder.build()
            },
        ),
    )
}
