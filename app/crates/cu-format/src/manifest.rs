// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};

use crate::error::FormatError;

pub const MANIFEST_VERSION: u32 = 1;

pub const MANIFEST_PATH: &str = ".cryptoupload/manifest";

pub const MANIFEST_DIR: &str = ".cryptoupload/manifests";

#[must_use]
pub fn manifest_object_path(seq: i64) -> String {
    format!("{MANIFEST_DIR}/{seq}")
}

#[must_use]
pub fn manifest_seq_from_name(name: &str) -> Option<i64> {
    let seq: i64 = name.parse().ok()?;
    if seq < 0 || seq.to_string() != name {
        return None;
    }
    Some(seq)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Writer {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "vaultPath")]
    pub vault_path: String,
    pub version: i64,
    #[serde(rename = "contentHash")]
    pub content_hash: String,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(rename = "cryptoUploadManifestVersion")]
    pub version: u32,
    pub seq: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    #[serde(rename = "lastWriter")]
    pub last_writer: Writer,
    pub entries: Vec<Entry>,
}

fn canonical_bytes(m: &Manifest) -> Vec<u8> {
    #[allow(clippy::expect_used)]
    let out = serde_json::to_vec(m).expect("manifest::Manifest is always serializable");
    out
}

#[must_use]
pub fn payload_hash(m: &Manifest) -> [u8; 32] {
    let mut hasher = cu_crypto::hash::Hasher::new();
    hasher.update(&canonical_bytes(m));
    hasher.finalize()
}

#[must_use]
pub fn mac(key: &cu_crypto::Secret<32>, m: &Manifest) -> [u8; 32] {
    cu_crypto::mac::hmac_sha256(key.expose(), &canonical_bytes(m))
}

#[must_use]
pub fn verify_mac(key: &cu_crypto::Secret<32>, m: &Manifest, candidate_mac: &[u8]) -> bool {
    cu_crypto::mac::verify(key.expose(), &canonical_bytes(m), candidate_mac)
}

#[must_use]
pub fn encode(m: &Manifest, mac_bytes: &[u8; 32]) -> Vec<u8> {
    #[derive(Serialize)]
    struct WithMac<'a> {
        #[serde(flatten)]
        manifest: &'a Manifest,
        mac: String,
    }
    let with_mac = WithMac {
        manifest: m,
        mac: hex_encode(mac_bytes),
    };
    #[allow(clippy::expect_used)]
    let out = serde_json::to_vec(&with_mac).expect("manifest::Manifest is always serializable");
    out
}

pub fn decode(bytes: &[u8]) -> Result<(Manifest, [u8; 32]), FormatError> {
    #[derive(Deserialize)]
    struct WithMac {
        #[serde(flatten)]
        manifest: Manifest,
        mac: String,
    }
    let with_mac: WithMac = serde_json::from_slice(bytes)
        .map_err(|_| FormatError::Malformed("manifest is not valid JSON"))?;
    if with_mac.manifest.version != MANIFEST_VERSION {
        return Err(FormatError::UnsupportedVersion(
            u8::try_from(with_mac.manifest.version).unwrap_or(u8::MAX),
        ));
    }
    let mac_bytes = hex_decode_32(&with_mac.mac)
        .ok_or(FormatError::Malformed("manifest mac is not 32-byte hex"))?;
    for entry in &with_mac.manifest.entries {
        if hex_decode_16(&entry.node_id).is_none() {
            return Err(FormatError::Malformed(
                "manifest entry nodeId is not 16-byte hex",
            ));
        }
        if hex_decode_16(&entry.device_id).is_none() {
            return Err(FormatError::Malformed(
                "manifest entry deviceId is not 16-byte hex",
            ));
        }
        if hex_decode_32(&entry.content_hash).is_none() {
            return Err(FormatError::Malformed(
                "manifest entry contentHash is not 32-byte hex",
            ));
        }
    }
    if hex_decode_16(&with_mac.manifest.last_writer.device_id).is_none() {
        return Err(FormatError::Malformed(
            "lastWriter deviceId is not 16-byte hex",
        ));
    }
    Ok((with_mac.manifest, mac_bytes))
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

pub(crate) fn hex_decode_n<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; N];
    #[allow(clippy::indexing_slicing)]
    for i in 0..N {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn hex_decode_16(s: &str) -> Option<[u8; 16]> {
    hex_decode_n::<16>(s)
}

fn hex_decode_32(s: &str) -> Option<[u8; 32]> {
    hex_decode_n::<32>(s)
}
