// SPDX-License-Identifier: GPL-3.0-or-later

use aes_kw::KekAes256;

use crate::error::CryptoError;
use crate::secret::Secret;

pub const OVERHEAD_LEN: usize = 8;

pub const WRAPPED_KEY_LEN: usize = 32 + OVERHEAD_LEN;

pub type WrappingKey = Secret<32>;

pub fn wrap(kek: &WrappingKey, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if plaintext.is_empty() || !plaintext.len().is_multiple_of(8) {
        return Err(CryptoError::InvalidParams(
            "keywrap: plaintext must be a non-empty multiple of 8 bytes",
        ));
    }
    let kek = KekAes256::from(*kek.expose());
    kek.wrap_vec(plaintext)
        .map_err(|_| CryptoError::InvalidParams("keywrap: wrapping failed"))
}

pub fn unwrap(
    kek: &WrappingKey,
    wrapped: &[u8],
) -> Result<zeroize::Zeroizing<Vec<u8>>, CryptoError> {
    let kek = KekAes256::from(*kek.expose());
    kek.unwrap_vec(wrapped)
        .map(zeroize::Zeroizing::new)
        .map_err(|_| CryptoError::DecryptFailed)
}
