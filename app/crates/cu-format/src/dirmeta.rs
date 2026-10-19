// SPDX-License-Identifier: GPL-3.0-or-later

use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};

use crate::error::FormatError;

pub const DIRMETA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    Decoy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub encrypted_name: String,
    pub kind: NodeKind,
    pub dir_id: Option<[u8; 32]>,
}

#[derive(Serialize, Deserialize)]
struct Payload {
    #[serde(rename = "cryptoUploadDirmetaVersion")]
    version: u32,
    entries: Vec<RawEntry>,
}

#[derive(Serialize, Deserialize)]
struct RawEntry {
    name: String,
    #[serde(rename = "encryptedName")]
    encrypted_name: String,
    kind: String,
    #[serde(rename = "dirId", skip_serializing_if = "Option::is_none", default)]
    dir_id: Option<String>,
}

#[must_use]
pub fn encode(entries: &[Entry]) -> Vec<u8> {
    let raw_entries = entries
        .iter()
        .map(|e| RawEntry {
            name: e.name.clone(),
            encrypted_name: e.encrypted_name.clone(),
            kind: match e.kind {
                NodeKind::File => "file".to_string(),
                NodeKind::Dir => "dir".to_string(),
                NodeKind::Decoy => "decoy".to_string(),
            },
            dir_id: e.dir_id.map(|id| BASE64URL_NOPAD.encode(&id)),
        })
        .collect();
    let payload = Payload {
        version: DIRMETA_VERSION,
        entries: raw_entries,
    };
    #[allow(clippy::expect_used)]
    let out = serde_json::to_vec(&payload).expect("dirmeta::Payload is always serializable");
    out
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Entry>, FormatError> {
    let payload: Payload = serde_json::from_slice(bytes)
        .map_err(|_| FormatError::Malformed("dirmeta.uvf is not valid JSON"))?;
    if payload.version != DIRMETA_VERSION {
        return Err(FormatError::UnsupportedVersion(
            u8::try_from(payload.version).unwrap_or(u8::MAX),
        ));
    }
    payload
        .entries
        .into_iter()
        .map(|raw| {
            let kind = match raw.kind.as_str() {
                "file" => NodeKind::File,
                "dir" => NodeKind::Dir,
                "decoy" => NodeKind::Decoy,
                _ => return Err(FormatError::Malformed("unknown kind in dirmeta.uvf")),
            };
            let dir_id = match (kind, raw.dir_id) {
                (NodeKind::Dir, Some(b64)) => {
                    let decoded = BASE64URL_NOPAD
                        .decode(b64.as_bytes())
                        .map_err(|_| FormatError::Malformed("dirId is not valid base64url"))?;
                    let arr: [u8; 32] = decoded
                        .try_into()
                        .map_err(|_| FormatError::Malformed("dirId is not 32 bytes"))?;
                    Some(arr)
                }
                (NodeKind::Dir, None) => {
                    return Err(FormatError::Malformed(
                        "dir entry without dirId in dirmeta.uvf",
                    ));
                }
                (NodeKind::File | NodeKind::Decoy, _) => None,
            };
            Ok(Entry {
                name: raw.name,
                encrypted_name: raw.encrypted_name,
                kind,
                dir_id,
            })
        })
        .collect()
}
