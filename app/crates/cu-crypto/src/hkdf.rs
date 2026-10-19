// SPDX-License-Identifier: GPL-3.0-or-later

use hkdf::Hkdf;
use sha2::Sha512;

use crate::error::CryptoError;
use crate::secret::Secret;

pub const MAX_OUTPUT_LEN: usize = 255 * 64;

pub fn derive(
    ikm: &[u8],
    salt: Option<&[u8]>,
    info: &[u8],
    out: &mut [u8],
) -> Result<(), CryptoError> {
    let hk = Hkdf::<Sha512>::new(salt, ikm);
    hk.expand(info, out)
        .map_err(|_| CryptoError::InvalidParams("hkdf: output longer than MAX_OUTPUT_LEN"))
}

pub fn derive_secret<const N: usize>(
    ikm: &[u8],
    salt: Option<&[u8]>,
    info: &[u8],
) -> Result<Secret<N>, CryptoError> {
    let mut out = [0u8; N];
    derive(ikm, salt, info, &mut out)?;
    Ok(Secret::new(out))
}
