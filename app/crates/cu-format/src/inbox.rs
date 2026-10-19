// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::Secret;
use cu_crypto::aead::{ContentKey, NONCE_LEN};
use cu_crypto::x25519::{StaticKeypair, ephemeral_diffie_hellman};

use crate::error::FormatError;

const MAGIC: &[u8; 4] = b"cuib";
pub const VERSION: u8 = 1;

const EPK_LEN: usize = 32;
const WRAPPED_KEY_LEN: usize = cu_crypto::keywrap::WRAPPED_KEY_LEN;

const MAGIC_OFFSET: usize = 0;
const VERSION_OFFSET: usize = MAGIC_OFFSET + 4;
const EPK_OFFSET: usize = VERSION_OFFSET + 1;
const WRAPPED_KEY_OFFSET: usize = EPK_OFFSET + EPK_LEN;
const CONTENT_NONCE_OFFSET: usize = WRAPPED_KEY_OFFSET + WRAPPED_KEY_LEN;
pub const HEADER_LEN: usize = CONTENT_NONCE_OFFSET + NONCE_LEN;

const ALGORITHM_ID: &[u8] = b"A256KW";
const KEK_BITS: u32 = 256;

pub const MAX_PROLOGUE_LEN: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Prologue {
    pub original_name: String,
    pub size: u64,
    pub mtime_ms: i64,
    pub depositor_label: String,
}

pub fn encode_prologue(prologue: &Prologue) -> Result<Vec<u8>, FormatError> {
    let json = serde_json::to_vec(prologue)
        .map_err(|_| FormatError::Malformed("inbox prologue cannot be serialized"))?;
    if json.len() > MAX_PROLOGUE_LEN {
        return Err(FormatError::Malformed(
            "inbox prologue exceeds the size limit",
        ));
    }
    let len = u32::try_from(json.len())
        .map_err(|_| FormatError::Malformed("inbox prologue too large"))?;
    let mut out = Vec::with_capacity(4 + json.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&json);
    Ok(out)
}

#[allow(clippy::indexing_slicing, clippy::expect_used)]
pub fn decode_prologue(buf: &[u8]) -> Result<Option<(Prologue, usize)>, FormatError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len_bytes: [u8; 4] = buf[..4].try_into().expect("the first 4 bytes always exist");
    let len = u32::from_be_bytes(len_bytes) as usize;
    if len > MAX_PROLOGUE_LEN {
        return Err(FormatError::Malformed(
            "inbox prologue declares an excessive length",
        ));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let prologue: Prologue = serde_json::from_slice(&buf[4..4 + len])
        .map_err(|_| FormatError::Malformed("inbox prologue is not valid JSON"))?;
    Ok(Some((prologue, 4 + len)))
}

pub fn seal_header(
    inbox_public_key: &[u8; EPK_LEN],
) -> Result<(Vec<u8>, ContentKey, [u8; NONCE_LEN]), FormatError> {
    let (epk_public, z) = ephemeral_diffie_hellman(inbox_public_key).map_err(|e| match e {
        cu_crypto::CryptoError::RandomUnavailable => FormatError::RandomUnavailable,
        _ => FormatError::InvalidInboxPublicKey,
    })?;
    let kek = derive_kek(z.expose(), &epk_public, inbox_public_key)?;

    let content_key = ContentKey::random()?;
    let wrapped_key = cu_crypto::keywrap::wrap(&kek, content_key.expose())?;

    let mut content_nonce = [0u8; NONCE_LEN];
    cu_crypto::random::fill_random(&mut content_nonce)?;

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(VERSION);
    header.extend_from_slice(&epk_public);
    header.extend_from_slice(&wrapped_key);
    header.extend_from_slice(&content_nonce);
    debug_assert_eq!(header.len(), HEADER_LEN);

    Ok((header, content_key, content_nonce))
}

#[allow(clippy::indexing_slicing, clippy::expect_used)]
pub fn open_header(
    bytes: &[u8],
    inbox_keypair: &StaticKeypair,
) -> Result<(ContentKey, [u8; NONCE_LEN]), FormatError> {
    if bytes.len() < HEADER_LEN {
        return Err(FormatError::DecryptFailed);
    }
    if &bytes[MAGIC_OFFSET..VERSION_OFFSET] != MAGIC {
        return Err(FormatError::Malformed(
            "first 4 bytes are not the inbox signature",
        ));
    }
    let version = bytes[VERSION_OFFSET];
    if version != VERSION {
        return Err(FormatError::UnsupportedVersion(version));
    }
    let epk: [u8; EPK_LEN] = bytes[EPK_OFFSET..WRAPPED_KEY_OFFSET]
        .try_into()
        .expect("fixed-length EPK_LEN slice");
    let wrapped_key = &bytes[WRAPPED_KEY_OFFSET..CONTENT_NONCE_OFFSET];
    let content_nonce: [u8; NONCE_LEN] = bytes[CONTENT_NONCE_OFFSET..HEADER_LEN]
        .try_into()
        .expect("HEADER_LEN - CONTENT_NONCE_OFFSET == NONCE_LEN by construction");

    let z = inbox_keypair
        .diffie_hellman(&epk)
        .map_err(|_| FormatError::DecryptFailed)?;
    let kek = derive_kek(z.expose(), &epk, &inbox_keypair.public_bytes())?;

    let content_key_bytes = cu_crypto::keywrap::unwrap(&kek, wrapped_key)?;
    let content_key_array: [u8; 32] = content_key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| FormatError::DecryptFailed)?;

    Ok((ContentKey::new(content_key_array), content_nonce))
}

fn derive_kek(
    z: &[u8],
    epk_public: &[u8; EPK_LEN],
    inbox_public_key: &[u8; EPK_LEN],
) -> Result<Secret<32>, FormatError> {
    let kek_bytes =
        cu_crypto::concat_kdf::derive(z, KEK_BITS, ALGORITHM_ID, epk_public, inbox_public_key)?;
    let kek_array: [u8; 32] = kek_bytes
        .as_slice()
        .try_into()
        .map_err(|_| FormatError::Malformed("inbox KEK is not 32 bytes"))?;
    Ok(Secret::new(kek_array))
}
