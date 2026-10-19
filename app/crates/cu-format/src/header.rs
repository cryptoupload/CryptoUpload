// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::Secret;
use cu_crypto::aead::{ContentKey, NONCE_LEN};

use crate::common::{SPEC_VERSION, SeedId};
use crate::error::FormatError;

pub const HEADER_LEN: usize = 68;

const MAGIC: &[u8; 3] = b"uvf";
const PREFIX_LEN: usize = 8;

#[allow(clippy::indexing_slicing, clippy::expect_used)]
pub fn seal(
    header_key: &Secret<32>,
    seed_id: SeedId,
) -> Result<(Vec<u8>, ContentKey, [u8; NONCE_LEN]), FormatError> {
    let file_key = ContentKey::random()?;

    let mut prefix = [0u8; PREFIX_LEN];
    prefix[..3].copy_from_slice(MAGIC);
    prefix[3] = SPEC_VERSION;
    prefix[4..8].copy_from_slice(seed_id.as_bytes());

    let sealed_key = cu_crypto::aead::seal_with_aad(header_key, &prefix, file_key.expose())?;
    let header_nonce: [u8; NONCE_LEN] = sealed_key[..NONCE_LEN]
        .try_into()
        .expect("seal_with_aad always prepends NONCE_LEN nonce bytes");

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(&prefix);
    header.extend_from_slice(&sealed_key);
    debug_assert_eq!(header.len(), HEADER_LEN);

    Ok((header, file_key, header_nonce))
}

#[allow(clippy::indexing_slicing, clippy::expect_used)]
pub fn open(
    bytes: &[u8],
    resolve_seed: impl FnOnce(SeedId) -> Option<Secret<32>>,
    derive_header_key: impl FnOnce(&Secret<32>) -> Result<Secret<32>, FormatError>,
) -> Result<(ContentKey, [u8; NONCE_LEN], SeedId), FormatError> {
    if bytes.len() < HEADER_LEN {
        return Err(FormatError::DecryptFailed);
    }
    if &bytes[..3] != MAGIC {
        return Err(FormatError::Malformed(
            "first 3 bytes are not the \"uvf\" signature",
        ));
    }
    let version = bytes[3];
    if version != SPEC_VERSION {
        return Err(FormatError::UnsupportedVersion(version));
    }
    let seed_id = SeedId(bytes[4..8].try_into().expect("4-byte slice"));
    let seed = resolve_seed(seed_id).ok_or(FormatError::DecryptFailed)?;
    let header_key = derive_header_key(&seed)?;

    let prefix = &bytes[..PREFIX_LEN];
    let sealed_key = &bytes[PREFIX_LEN..HEADER_LEN];
    let header_nonce: [u8; NONCE_LEN] = sealed_key[..NONCE_LEN]
        .try_into()
        .expect("HEADER_LEN - PREFIX_LEN is always >= NONCE_LEN");

    let file_key_bytes = zeroize::Zeroizing::new(cu_crypto::aead::open_with_aad(
        &header_key,
        prefix,
        sealed_key,
    )?);
    let file_key_bytes: [u8; 32] = file_key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| FormatError::DecryptFailed)?;

    Ok((ContentKey::new(file_key_bytes), header_nonce, seed_id))
}
