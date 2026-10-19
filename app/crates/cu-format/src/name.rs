// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::siv::NameKey;
use std::borrow::Cow;

use data_encoding::BASE64URL_NOPAD;
use unicode_normalization::UnicodeNormalization;

use crate::config::FormatProfile;
use crate::error::FormatError;

pub const SUFFIX: &str = ".uvf";

const MAX_PATH_COMPONENT_BYTES: usize = 255;

#[must_use]
pub fn nfc(name: &str) -> Cow<'_, str> {
    if name.is_ascii() || unicode_normalization::is_nfc(name) {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(name.nfc().collect())
    }
}

pub fn encrypt(key: &NameKey, dir_id: &[u8], name: &str) -> Result<String, FormatError> {
    let ciphertext = cu_crypto::siv::encrypt_name(key, dir_id, nfc(name).as_bytes())?;
    let encoded = BASE64URL_NOPAD.encode(&ciphertext);
    let full = format!("{encoded}{SUFFIX}");
    if full.len() > MAX_PATH_COMPONENT_BYTES {
        return Err(FormatError::NameTooLong);
    }
    Ok(full)
}

pub const NAME_PAD_BLOCK: usize = 32;

pub const MAX_PADDED_NAME_BYTES: usize = 160;

pub const DECOY_MARKER: char = '\u{1}';

#[must_use]
pub fn is_decoy(name: &str) -> bool {
    name.starts_with(DECOY_MARKER)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unstorable {
    TooLong,
    TooLongPrivate,
    ReservedByte,
}

#[must_use]
pub fn unstorable_reason(profile: FormatProfile, name: &str) -> Option<Unstorable> {
    match profile {
        FormatProfile::Standard => {
            let encoded = (name.len() + 16).div_ceil(3) * 4 - (3 - (name.len() + 16) % 3) % 3;
            (encoded + SUFFIX.len() > MAX_PATH_COMPONENT_BYTES).then_some(Unstorable::TooLong)
        }
        FormatProfile::Private => {
            if name.len() > MAX_PADDED_NAME_BYTES {
                Some(Unstorable::TooLongPrivate)
            } else if name.contains('\0') || is_decoy(name) {
                Some(Unstorable::ReservedByte)
            } else {
                None
            }
        }
    }
}

pub fn encrypt_in(
    profile: FormatProfile,
    key: &NameKey,
    dir_id: &[u8],
    name: &str,
) -> Result<String, FormatError> {
    match profile {
        FormatProfile::Standard => encrypt(key, dir_id, name),
        FormatProfile::Private => {
            if name.contains('\0') || is_decoy(name) {
                return Err(FormatError::ReservedName);
            }
            encrypt_padded(key, dir_id, name)
        }
    }
}

pub fn encrypt_decoy(key: &NameKey, dir_id: &[u8], name: &str) -> Result<String, FormatError> {
    if !is_decoy(name) || name.contains('\0') {
        return Err(FormatError::ReservedName);
    }
    encrypt_padded(key, dir_id, name)
}

fn encrypt_padded(key: &NameKey, dir_id: &[u8], name: &str) -> Result<String, FormatError> {
    let normalized = nfc(name);
    if normalized.len() > MAX_PADDED_NAME_BYTES {
        return Err(FormatError::NameTooLong);
    }
    let mut padded = normalized.as_bytes().to_vec();
    padded.resize(padded.len().next_multiple_of(NAME_PAD_BLOCK), 0);
    let ciphertext = cu_crypto::siv::encrypt_name(key, dir_id, &padded)?;
    let full = format!("{}{SUFFIX}", BASE64URL_NOPAD.encode(&ciphertext));
    if full.len() > MAX_PATH_COMPONENT_BYTES {
        return Err(FormatError::NameTooLong);
    }
    Ok(full)
}

pub fn decrypt_in(
    profile: FormatProfile,
    key: &NameKey,
    dir_id: &[u8],
    ciphertext_name: &str,
) -> Result<String, FormatError> {
    if profile == FormatProfile::Standard {
        return decrypt(key, dir_id, ciphertext_name);
    }
    let encoded = ciphertext_name
        .strip_suffix(SUFFIX)
        .ok_or(FormatError::Malformed("name without .uvf extension"))?;
    let ciphertext = BASE64URL_NOPAD
        .decode(encoded.as_bytes())
        .map_err(|_| FormatError::Malformed("name is not valid base64url"))?;
    let mut plaintext = cu_crypto::siv::decrypt_name(key, dir_id, &ciphertext)?;
    if plaintext.is_empty() || plaintext.len() % NAME_PAD_BLOCK != 0 {
        return Err(FormatError::Malformed("name padding not valid"));
    }
    while plaintext.last() == Some(&0) {
        plaintext.pop();
    }
    let name = String::from_utf8(plaintext).map_err(|_| FormatError::DecryptFailed)?;
    if !is_single_component(&name) {
        return Err(FormatError::Malformed(
            "decrypted name is not a single path component",
        ));
    }
    Ok(name)
}

pub fn decrypt(key: &NameKey, dir_id: &[u8], ciphertext_name: &str) -> Result<String, FormatError> {
    let encoded = ciphertext_name
        .strip_suffix(SUFFIX)
        .ok_or(FormatError::Malformed("name without .uvf extension"))?;
    let ciphertext = BASE64URL_NOPAD
        .decode(encoded.as_bytes())
        .map_err(|_| FormatError::Malformed("name is not valid base64url"))?;
    let plaintext = cu_crypto::siv::decrypt_name(key, dir_id, &ciphertext)?;
    let name = String::from_utf8(plaintext).map_err(|_| FormatError::DecryptFailed)?;
    if !is_single_component(&name) {
        return Err(FormatError::Malformed(
            "decrypted name is not a single path component",
        ));
    }
    Ok(name)
}

#[must_use]
pub fn is_single_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0'])
}
