// SPDX-License-Identifier: GPL-3.0-or-later

use aes_gcm::aead::{Aead, AeadInPlace, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};

use crate::error::CryptoError;
use crate::secret::Secret;

pub const PLAINTEXT_CHUNK: usize = 32_740;
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
pub const CIPHERTEXT_CHUNK: usize = NONCE_LEN + PLAINTEXT_CHUNK + TAG_LEN;

const _: () = assert!(
    CIPHERTEXT_CHUNK == 32_768,
    "the chunk geometry is part of the vault format and must not change"
);

const AAD_LEN: usize = 8 + NONCE_LEN;

pub type ContentKey = Secret<32>;

pub fn seal_chunk(
    content_key: &ContentKey,
    chunk_index: u64,
    header_nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if plaintext.len() > PLAINTEXT_CHUNK {
        return Err(CryptoError::InvalidParams(
            "aead: plaintext longer than PLAINTEXT_CHUNK",
        ));
    }

    let mut nonce_bytes = [0u8; NONCE_LEN];
    crate::random::fill_random(&mut nonce_bytes)?;

    let cipher = Aes256Gcm::new_from_slice(content_key.expose())
        .map_err(|_| CryptoError::InvalidParams("aead: unexpected key length"))?;
    let aad = chunk_aad(chunk_index, header_nonce);

    let mut sealed = Vec::with_capacity(NONCE_LEN + plaintext.len() + TAG_LEN);
    sealed.extend_from_slice(&nonce_bytes);
    sealed.extend_from_slice(plaintext);
    #[allow(clippy::indexing_slicing)]
    let tag = cipher
        .encrypt_in_place_detached(
            Nonce::from_slice(&nonce_bytes),
            &aad,
            &mut sealed[NONCE_LEN..],
        )
        .map_err(|_| CryptoError::InvalidParams("aead: encryption failed"))?;
    sealed.extend_from_slice(&tag);
    Ok(sealed)
}

pub fn open_chunk(
    content_key: &ContentKey,
    chunk_index: u64,
    header_nonce: &[u8; NONCE_LEN],
    sealed: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::DecryptFailed);
    }
    let (nonce_bytes, rest) = sealed.split_at(NONCE_LEN);

    let cipher =
        Aes256Gcm::new_from_slice(content_key.expose()).map_err(|_| CryptoError::DecryptFailed)?;
    let aad = chunk_aad(chunk_index, header_nonce);
    cipher
        .decrypt(
            Nonce::from_slice(nonce_bytes),
            Payload {
                msg: rest,
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::DecryptFailed)
}

fn chunk_aad(chunk_index: u64, header_nonce: &[u8; NONCE_LEN]) -> [u8; AAD_LEN] {
    let mut aad = [0u8; AAD_LEN];
    aad[..8].copy_from_slice(&chunk_index.to_be_bytes());
    aad[8..].copy_from_slice(header_nonce);
    aad
}

pub fn seal_with_aad(
    key: &Secret<32>,
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    crate::random::fill_random(&mut nonce_bytes)?;

    let cipher = Aes256Gcm::new_from_slice(key.expose())
        .map_err(|_| CryptoError::InvalidParams("aead: unexpected key length"))?;

    let mut sealed = Vec::with_capacity(NONCE_LEN + plaintext.len() + TAG_LEN);
    sealed.extend_from_slice(&nonce_bytes);
    sealed.extend_from_slice(plaintext);
    #[allow(clippy::indexing_slicing)]
    let tag = cipher
        .encrypt_in_place_detached(
            Nonce::from_slice(&nonce_bytes),
            aad,
            &mut sealed[NONCE_LEN..],
        )
        .map_err(|_| CryptoError::InvalidParams("aead: encryption failed"))?;
    sealed.extend_from_slice(&tag);
    Ok(sealed)
}

pub fn open_with_aad(key: &Secret<32>, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::DecryptFailed);
    }
    let (nonce_bytes, rest) = sealed.split_at(NONCE_LEN);

    let cipher = Aes256Gcm::new_from_slice(key.expose()).map_err(|_| CryptoError::DecryptFailed)?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), Payload { msg: rest, aad })
        .map_err(|_| CryptoError::DecryptFailed)
}
