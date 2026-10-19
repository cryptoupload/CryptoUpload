// SPDX-License-Identifier: GPL-3.0-or-later

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const OUTPUT_LEN: usize = 32;

#[must_use]
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; OUTPUT_LEN] {
    #[allow(clippy::expect_used)]
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC-SHA256 accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

#[must_use]
pub fn verify(key: &[u8], data: &[u8], expected: &[u8]) -> bool {
    #[allow(clippy::expect_used)]
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key).expect("HMAC-SHA256 accepts keys of any length");
    mac.update(data);
    mac.verify_slice(expected).is_ok()
}
