// SPDX-License-Identifier: GPL-3.0-or-later

use sha2::{Digest, Sha256};

use crate::error::CryptoError;

pub const MAX_SINGLE_ROUND_KEYDATALEN_BITS: u32 = 256;

#[allow(clippy::similar_names)]
pub fn derive(
    z: &[u8],
    keydatalen_bits: u32,
    algorithm_id: &[u8],
    party_u_info: &[u8],
    party_v_info: &[u8],
) -> Result<zeroize::Zeroizing<Vec<u8>>, CryptoError> {
    if keydatalen_bits == 0
        || !keydatalen_bits.is_multiple_of(8)
        || keydatalen_bits > MAX_SINGLE_ROUND_KEYDATALEN_BITS
    {
        return Err(CryptoError::InvalidParams(
            "concat_kdf: keydatalen_bits must be a multiple of 8, between 8 and 256",
        ));
    }

    let mut hasher = Sha256::new();
    hasher.update(1u32.to_be_bytes());
    hasher.update(z);
    hasher.update((datalen(algorithm_id.len())?).to_be_bytes());
    hasher.update(algorithm_id);
    hasher.update((datalen(party_u_info.len())?).to_be_bytes());
    hasher.update(party_u_info);
    hasher.update((datalen(party_v_info.len())?).to_be_bytes());
    hasher.update(party_v_info);
    hasher.update(keydatalen_bits.to_be_bytes());
    let digest = hasher.finalize();

    let keydatalen_bytes = (keydatalen_bits / 8) as usize;
    #[allow(clippy::indexing_slicing)]
    let out = digest[..keydatalen_bytes].to_vec();
    Ok(zeroize::Zeroizing::new(out))
}

fn datalen(len: usize) -> Result<u32, CryptoError> {
    u32::try_from(len)
        .map_err(|_| CryptoError::InvalidParams("concat_kdf: OtherInfo field longer than 4 GiB"))
}
