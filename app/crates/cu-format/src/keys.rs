// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::Secret;
use cu_crypto::hkdf::derive_secret;

use crate::error::FormatError;

pub fn header_key(seed: &Secret<32>, salt: &[u8; 32]) -> Result<Secret<32>, FormatError> {
    Ok(derive_secret(seed.expose(), Some(salt), b"fileHeader")?)
}

pub fn siv_key(seed: &Secret<32>, salt: &[u8; 32]) -> Result<Secret<64>, FormatError> {
    Ok(derive_secret(seed.expose(), Some(salt), b"siv")?)
}

pub fn hmac_key(seed: &Secret<32>, salt: &[u8; 32]) -> Result<Secret<64>, FormatError> {
    Ok(derive_secret(seed.expose(), Some(salt), b"hmac")?)
}

pub fn root_dir_id(initial_seed: &Secret<32>, salt: &[u8; 32]) -> Result<[u8; 32], FormatError> {
    let secret: Secret<32> = derive_secret(initial_seed.expose(), Some(salt), b"rootDirId")?;
    Ok(*secret.expose())
}

pub fn local_state_key(
    initial_seed: &Secret<32>,
    salt: &[u8; 32],
) -> Result<Secret<64>, FormatError> {
    Ok(derive_secret(
        initial_seed.expose(),
        Some(salt),
        b"org.cryptoupload.localState.v1",
    )?)
}

pub fn dirmeta_header_key(seed: &Secret<32>, salt: &[u8; 32]) -> Result<Secret<32>, FormatError> {
    Ok(derive_secret(
        seed.expose(),
        Some(salt),
        b"org.cryptoupload.dirmeta.v1",
    )?)
}

pub fn manifest_mac_key(
    initial_seed: &Secret<32>,
    salt: &[u8; 32],
) -> Result<Secret<32>, FormatError> {
    Ok(derive_secret(
        initial_seed.expose(),
        Some(salt),
        b"org.cryptoupload.manifest.v1",
    )?)
}

pub fn inbox_seed(initial_seed: &Secret<32>, salt: &[u8; 32]) -> Result<Secret<32>, FormatError> {
    Ok(derive_secret(
        initial_seed.expose(),
        Some(salt),
        b"org.cryptoupload.inbox.v1",
    )?)
}
