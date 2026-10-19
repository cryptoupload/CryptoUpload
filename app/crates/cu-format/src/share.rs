// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::aead::{ContentKey, NONCE_LEN};
use cu_crypto::kdf::{KdfParams, derive_kek};

use crate::error::FormatError;

const MAGIC: &[u8; 4] = b"cush";
pub const VERSION: u8 = 1;

const SALT_LEN: usize = 16;
const WRAPPED_KEY_LEN: usize = cu_crypto::keywrap::WRAPPED_KEY_LEN;

const MAGIC_OFFSET: usize = 0;
const VERSION_OFFSET: usize = MAGIC_OFFSET + 4;
const PARAMS_OFFSET: usize = VERSION_OFFSET + 1;
const SALT_OFFSET: usize = PARAMS_OFFSET + 12;
const WRAPPED_KEY_OFFSET: usize = SALT_OFFSET + SALT_LEN;
const CONTENT_NONCE_OFFSET: usize = WRAPPED_KEY_OFFSET + WRAPPED_KEY_LEN;
pub const HEADER_LEN: usize = CONTENT_NONCE_OFFSET + NONCE_LEN;

pub fn seal_header(
    password: &[u8],
    params: KdfParams,
) -> Result<(Vec<u8>, ContentKey, [u8; NONCE_LEN]), FormatError> {
    let mut salt = [0u8; SALT_LEN];
    cu_crypto::random::fill_random(&mut salt)?;
    let kek = derive_kek(password, &salt, params)?;

    let content_key = ContentKey::random()?;
    let wrapped_key = cu_crypto::keywrap::wrap(&kek, content_key.expose())?;

    let mut content_nonce = [0u8; NONCE_LEN];
    cu_crypto::random::fill_random(&mut content_nonce)?;

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(VERSION);
    header.extend_from_slice(&params.m_cost_kib().to_be_bytes());
    header.extend_from_slice(&params.t_cost().to_be_bytes());
    header.extend_from_slice(&params.p_cost().to_be_bytes());
    header.extend_from_slice(&salt);
    header.extend_from_slice(&wrapped_key);
    header.extend_from_slice(&content_nonce);
    debug_assert_eq!(header.len(), HEADER_LEN);

    Ok((header, content_key, content_nonce))
}

#[allow(clippy::indexing_slicing, clippy::expect_used)]
pub fn open_header(
    bytes: &[u8],
    password: &[u8],
) -> Result<(ContentKey, [u8; NONCE_LEN]), FormatError> {
    if bytes.len() < HEADER_LEN {
        return Err(FormatError::DecryptFailed);
    }
    if &bytes[MAGIC_OFFSET..VERSION_OFFSET] != MAGIC {
        return Err(FormatError::Malformed(
            "first 4 bytes are not a share package signature",
        ));
    }
    let version = bytes[VERSION_OFFSET];
    if version != VERSION {
        return Err(FormatError::UnsupportedVersion(version));
    }
    let read_u32 = |offset: usize| {
        u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("4-byte slice within HEADER_LEN"),
        )
    };
    let params = KdfParams::from_untrusted(
        read_u32(PARAMS_OFFSET),
        read_u32(PARAMS_OFFSET + 4),
        read_u32(PARAMS_OFFSET + 8),
    )?;
    let salt = &bytes[SALT_OFFSET..WRAPPED_KEY_OFFSET];
    let wrapped_key = &bytes[WRAPPED_KEY_OFFSET..CONTENT_NONCE_OFFSET];
    let content_nonce: [u8; NONCE_LEN] = bytes[CONTENT_NONCE_OFFSET..HEADER_LEN]
        .try_into()
        .expect("HEADER_LEN - CONTENT_NONCE_OFFSET == NONCE_LEN by construction");

    let kek = derive_kek(password, salt, params)?;
    let content_key_bytes = cu_crypto::keywrap::unwrap(&kek, wrapped_key)?;
    let content_key_array: [u8; 32] = content_key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| FormatError::DecryptFailed)?;

    Ok((ContentKey::new(content_key_array), content_nonce))
}
