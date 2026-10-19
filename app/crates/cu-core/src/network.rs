// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::{Path, PathBuf};

use cu_backend::proxy::Proxy;

use crate::error::CoreError;

#[derive(serde::Serialize, serde::Deserialize)]
struct NetworkFile {
    #[serde(default)]
    proxy: Option<String>,
}

#[must_use]
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join("network.json")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyStatus {
    Direct,
    Active(String),
    Broken(Msg),
}

fn read(data_dir: &Path) -> Result<Option<Proxy>, Msg> {
    let path = settings_path(data_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Msg::new("PROXY_SETTINGS_UNREADABLE").arg(e)),
    };
    let file: NetworkFile =
        serde_json::from_slice(&bytes).map_err(|_| Msg::new("PROXY_SETTINGS_INVALID"))?;
    file.proxy.as_deref().map(Proxy::parse).transpose()
}

fn apply(proxy: Option<Proxy>) {
    cu_oauth::set_proxy_url(proxy.as_ref().map(Proxy::url));
    cu_backend::proxy::set_global(proxy);
}

#[allow(clippy::must_use_candidate)]
pub fn apply_saved_proxy(data_dir: &Path) -> ProxyStatus {
    match read(data_dir) {
        Ok(None) => {
            apply(None);
            ProxyStatus::Direct
        }
        Ok(Some(proxy)) => {
            let url = proxy.url();
            apply(Some(proxy));
            ProxyStatus::Active(url)
        }
        Err(reason) => {
            apply(Some(Proxy::unreachable()));
            ProxyStatus::Broken(reason)
        }
    }
}

#[must_use]
pub fn proxy_status(data_dir: &Path) -> ProxyStatus {
    match read(data_dir) {
        Ok(None) => ProxyStatus::Direct,
        Ok(Some(proxy)) => ProxyStatus::Active(proxy.url()),
        Err(reason) => ProxyStatus::Broken(reason),
    }
}

pub fn set_proxy(data_dir: &Path, url: Option<&str>) -> Result<ProxyStatus, CoreError> {
    let proxy = url
        .map(Proxy::parse)
        .transpose()
        .map_err(CoreError::InvalidProxy)?;
    let path = settings_path(data_dir);
    match &proxy {
        Some(proxy) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let json = serde_json::to_vec_pretty(&NetworkFile {
                proxy: Some(proxy.url()),
            })
            .map_err(|e| CoreError::Io(std::io::Error::other(e)))?;
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, json)?;
            std::fs::rename(&tmp, &path)?;
        }
        None => match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        },
    }
    let status = proxy
        .as_ref()
        .map_or(ProxyStatus::Direct, |p| ProxyStatus::Active(p.url()));
    apply(proxy);
    Ok(status)
}
