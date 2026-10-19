// SPDX-License-Identifier: GPL-3.0-or-later

use aes_siv::KeyInit;
use aes_siv::siv::Aes256Siv;

use crate::error::CryptoError;
use crate::secret::Secret;

pub type NameKey = Secret<64>;

pub fn encrypt_name(key: &NameKey, dir_id: &[u8], name: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let key_arr = aes_siv::Key::<Aes256Siv>::from_slice(key.expose());
    let mut cipher = Aes256Siv::new(key_arr);
    cipher
        .encrypt([dir_id], name)
        .map_err(|_| CryptoError::InvalidParams("aes-siv: too many AAD components"))
}

pub fn decrypt_name(
    key: &NameKey,
    dir_id: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let key_arr = aes_siv::Key::<Aes256Siv>::from_slice(key.expose());
    let mut cipher = Aes256Siv::new(key_arr);
    cipher
        .decrypt([dir_id], ciphertext)
        .map_err(|_| CryptoError::DecryptFailed)
}
