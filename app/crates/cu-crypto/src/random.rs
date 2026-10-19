// SPDX-License-Identifier: GPL-3.0-or-later

use crate::error::CryptoError;

pub fn fill_random(buf: &mut [u8]) -> Result<(), CryptoError> {
    getrandom::getrandom(buf).map_err(|_| CryptoError::RandomUnavailable)
}
