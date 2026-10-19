// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};

#[derive(Debug)]
pub enum SecretsError {
    Io(std::io::Error),
    Malformed(&'static str),
    UnsupportedVersion(u8),
    DecryptFailed,
    Crypto(cu_crypto::CryptoError),
    Json(serde_json::Error),
    Keychain(String),
    KeychainTimeout(u64),
    NoSuchEntry,
    LegacyFormat,
    IdentityExists,
}

impl Coded for SecretsError {
    fn msg(&self) -> Msg {
        match self {
            Self::Io(e) => Msg::new("IO").arg(e),
            Self::Malformed(why) => Msg::new("SECRETS_MALFORMED").arg(why),
            Self::UnsupportedVersion(v) => Msg::new("SECRETS_UNSUPPORTED_VERSION").arg(v),
            Self::DecryptFailed => Msg::new("SECRETS_DECRYPT_FAILED"),
            Self::Crypto(e) => e.msg(),
            Self::Json(e) => Msg::new("SECRETS_JSON").arg(e),
            Self::Keychain(why) => Msg::new("SECRETS_KEYCHAIN").arg(why),
            Self::KeychainTimeout(secs) => Msg::new("SECRETS_KEYCHAIN_TIMEOUT").arg(secs),
            Self::NoSuchEntry => Msg::new("SECRETS_NO_SUCH_ENTRY"),
            Self::LegacyFormat => Msg::new("SECRETS_LEGACY_FORMAT"),
            Self::IdentityExists => Msg::new("SECRETS_IDENTITY_EXISTS"),
        }
    }
}

display_as_msg!(SecretsError);

impl std::error::Error for SecretsError {}

impl From<std::io::Error> for SecretsError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<cu_crypto::CryptoError> for SecretsError {
    fn from(err: cu_crypto::CryptoError) -> Self {
        match err {
            cu_crypto::CryptoError::DecryptFailed => Self::DecryptFailed,
            other => Self::Crypto(other),
        }
    }
}

impl From<serde_json::Error> for SecretsError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}
