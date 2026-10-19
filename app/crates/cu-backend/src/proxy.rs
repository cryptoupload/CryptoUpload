// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    Socks5,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    kind: ProxyKind,
    host: String,
    port: u16,
}

impl Proxy {
    pub fn parse(url: &str) -> Result<Self, Msg> {
        let url = url.trim();
        let (kind, rest) = if let Some(rest) = url
            .strip_prefix("socks5h://")
            .or_else(|| url.strip_prefix("socks5://"))
        {
            (ProxyKind::Socks5, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (ProxyKind::Http, rest)
        } else {
            return Err(Msg::new("PROXY_SCHEME"));
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains('@') {
            return Err(Msg::new("PROXY_CREDENTIALS"));
        }
        let (host, port) = rest
            .rsplit_once(':')
            .ok_or_else(|| Msg::new("PROXY_PORT_MISSING"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let valid_host = !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
        if !valid_host {
            return Err(Msg::new("PROXY_HOST_INVALID"));
        }
        let port: u16 = port
            .parse()
            .ok()
            .filter(|&p| p != 0)
            .ok_or_else(|| Msg::new("PROXY_PORT_INVALID"))?;
        Ok(Self {
            kind,
            host: host.to_string(),
            port,
        })
    }

    #[must_use]
    pub fn unreachable() -> Self {
        Self {
            kind: ProxyKind::Http,
            host: "127.0.0.1".to_string(),
            port: 1,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ProxyKind {
        self.kind
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    #[must_use]
    pub fn url(&self) -> String {
        let scheme = match self.kind {
            ProxyKind::Socks5 => "socks5h",
            ProxyKind::Http => "http",
        };
        if self.host.contains(':') {
            format!("{scheme}://[{}]:{}", self.host, self.port)
        } else {
            format!("{scheme}://{}:{}", self.host, self.port)
        }
    }
}

static GLOBAL: RwLock<Option<Proxy>> = RwLock::new(None);

pub fn set_global(proxy: Option<Proxy>) {
    *GLOBAL
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = proxy;
}

#[must_use]
pub fn global() -> Option<Proxy> {
    GLOBAL
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}
