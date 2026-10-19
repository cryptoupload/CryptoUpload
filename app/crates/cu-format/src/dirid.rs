// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::Secret;
use data_encoding::BASE32_NOPAD;

use crate::error::FormatError;
use crate::keys;

pub const DIR_ID_LEN: usize = 32;

pub fn random_dir_id() -> Result<[u8; DIR_ID_LEN], FormatError> {
    let mut buf = [0u8; DIR_ID_LEN];
    cu_crypto::random::fill_random(&mut buf)?;
    Ok(buf)
}

pub fn root_dir_id(
    initial_seed: &Secret<32>,
    kdf_salt: &[u8; 32],
) -> Result<[u8; DIR_ID_LEN], FormatError> {
    keys::root_dir_id(initial_seed, kdf_salt)
}

pub fn physical_path(
    dir_id: &[u8; DIR_ID_LEN],
    dir_seed: &Secret<32>,
    kdf_salt: &[u8; 32],
) -> Result<String, FormatError> {
    let hmac_key = keys::hmac_key(dir_seed, kdf_salt)?;
    let tag = cu_crypto::mac::hmac_sha256(&hmac_key.expose()[..32], dir_id);
    let truncated = &tag[..20];
    let encoded = BASE32_NOPAD.encode(truncated);
    debug_assert_eq!(encoded.len(), 32, "160 bits in Base32 are always 32 chars");
    Ok(format!("d/{}/{}", &encoded[..2], &encoded[2..]))
}
