// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::Secret;
use cu_format::config::RecipientInfo;
use cu_msg::{Coded, Msg};
use data_encoding::BASE64URL_NOPAD;

use crate::error::CoreError;

pub const FIDO2_RP_ID: &str = "cryptoupload.local";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockMethod {
    Password,
    RecoverySeed,
    Fido2 { rp_id: String },
    Platform,
    Member { name: String, public_key: [u8; 32] },
    TwoFactor { rp_id: String },
    Unknown(String),
}

impl From<RecipientInfo> for UnlockMethod {
    fn from(info: RecipientInfo) -> Self {
        match info {
            RecipientInfo::Password => Self::Password,
            RecipientInfo::RecoverySeed => Self::RecoverySeed,
            RecipientInfo::Fido2 { rp_id, .. } => Self::Fido2 { rp_id },
            RecipientInfo::TwoFactor { rp_id, .. } => Self::TwoFactor { rp_id },
            RecipientInfo::Platform { .. } => Self::Platform,
            RecipientInfo::Member { name, public_key } => Self::Member { name, public_key },
            RecipientInfo::Unknown(kid) => Self::Unknown(kid),
        }
    }
}

const PLATFORM_KEYCHAIN_SERVICE: &str = "cryptoupload-platform-unlock";

const CONSENT_REASON: &str = "Unlock the CryptoUpload vault";

static CONSENT_REASON_TEXT: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

pub fn set_consent_reason(text: String) {
    *CONSENT_REASON_TEXT
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(text);
}

pub fn set_oauth_pages(granted: String, denied: String) {
    cu_oauth::set_consent_pages(granted, denied);
}

fn consent_reason() -> String {
    CONSENT_REASON_TEXT
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| CONSENT_REASON.to_string())
}

fn platform_keychain_account(keychain_id: &[u8; 16]) -> String {
    BASE64URL_NOPAD.encode(keychain_id)
}

pub(crate) fn generate_and_store_kek() -> Result<([u8; 16], Secret<32>), CoreError> {
    let mut keychain_id = [0u8; 16];
    cu_crypto::random::fill_random(&mut keychain_id).map_err(|_| CoreError::RandomUnavailable)?;
    let kek = Secret::<32>::random().map_err(|_| CoreError::RandomUnavailable)?;
    let encoded = BASE64URL_NOPAD.encode(kek.expose());
    cu_secrets::keychain::store_password(
        PLATFORM_KEYCHAIN_SERVICE,
        &platform_keychain_account(&keychain_id),
        &encoded,
    )
    .map_err(|e| CoreError::PlatformKeychain(e.msg()))?;
    Ok((keychain_id, kek))
}

pub(crate) fn load_kek(keychain_id: &[u8; 16]) -> Result<Option<Secret<32>>, CoreError> {
    let account = platform_keychain_account(keychain_id);
    let encoded = cu_secrets::keychain::load_password_with_consent(
        PLATFORM_KEYCHAIN_SERVICE,
        &account,
        &consent_reason(),
    )
    .map_err(|e| CoreError::PlatformKeychain(e.msg()))?;
    let Some(encoded) = encoded else {
        return Ok(None);
    };
    let bytes = BASE64URL_NOPAD
        .decode(encoded.as_bytes())
        .map_err(|_| CoreError::PlatformKeychain(Msg::new("KEYCHAIN_ENTRY_INVALID")))?;
    let kek: [u8; 32] = bytes
        .try_into()
        .map_err(|_| CoreError::PlatformKeychain(Msg::new("KEYCHAIN_ENTRY_NOT_KEK")))?;
    Ok(Some(Secret::new(kek)))
}

pub(crate) fn forget_kek(keychain_id: &[u8; 16]) -> Result<(), CoreError> {
    cu_secrets::keychain::delete_password(
        PLATFORM_KEYCHAIN_SERVICE,
        &platform_keychain_account(keychain_id),
    )
    .map_err(|e| CoreError::PlatformKeychain(e.msg()))
}

pub(crate) fn register_fido2(
    authenticator: &dyn cu_auth_fido2::Fido2Authenticator,
    rp_id: &str,
    pin: Option<&str>,
) -> Result<(Vec<u8>, [u8; 32], Secret<32>), CoreError> {
    let mut salt = [0u8; 32];
    cu_crypto::random::fill_random(&mut salt).map_err(|_| CoreError::RandomUnavailable)?;
    let (credential_id, hmac_secret) = authenticator
        .register(rp_id, &salt, pin)
        .map_err(|e| CoreError::Fido2(e.msg()))?;
    Ok((credential_id, salt, Secret::new(hmac_secret)))
}
