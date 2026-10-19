// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use crate::error::CoreError;

const FILE_NAME: &str = "device.json";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub label: String,
}

fn device_json_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join(FILE_NAME)
}

pub fn current(data_dir: &Path) -> Result<DeviceIdentity, CoreError> {
    let path = device_json_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).map_err(|e| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{FILE_NAME}: {e}"),
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let identity = generate()?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let payload = serde_json::to_vec_pretty(&identity)
                .map_err(|e| CoreError::Io(std::io::Error::from(e)))?;
            let tmp = path.with_extension(format!(
                "json.tmp.{}.{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos())
            ));
            {
                use std::io::Write as _;
                let mut file = std::fs::File::create(&tmp)?;
                file.write_all(&payload)?;
                file.sync_all()?;
            }
            std::fs::rename(&tmp, &path)?;
            Ok(identity)
        }
        Err(e) => Err(e.into()),
    }
}

fn generate() -> Result<DeviceIdentity, CoreError> {
    let mut buf = [0u8; 16];
    cu_crypto::random::fill_random(&mut buf).map_err(|_| CoreError::RandomUnavailable)?;
    let device_id = hex(&buf);
    let label = hostname_label().unwrap_or_else(|| format!("device-{}", &device_id[..8]));
    Ok(DeviceIdentity { device_id, label })
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

fn hostname_label() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
}
