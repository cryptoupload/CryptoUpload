// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    DecryptFailed,
    UnsupportedVersion(u8),
    UnsupportedPolicyVersion(u32),
    UnknownAlgorithm(&'static str, String),
    Malformed(&'static str),
    NameTooLong,
    ReservedName,
    NotInThisProfile(&'static str),
    UnsupportedNodeType,
    RandomUnavailable,
    NoRecoveryRecipient,
    NoFido2Recipient,
    NoPlatformRecipient,
    NoMemberRecipient,
    InvalidMemberPublicKey,
    TwoFactorRequired,
    NoSuchRecipient,
    LastRecipient,
    RekeyCredentialRequired(&'static str),
    InvalidInboxPublicKey,
}

impl Coded for FormatError {
    fn msg(&self) -> Msg {
        match self {
            Self::DecryptFailed => Msg::new("DECRYPT_FAILED"),
            Self::UnsupportedVersion(v) => Msg::new("FORMAT_UNSUPPORTED_VERSION").arg(v),
            Self::UnsupportedPolicyVersion(v) => Msg::new("FORMAT_UNSUPPORTED_POLICY").arg(v),
            Self::UnknownAlgorithm(field, value) => {
                Msg::new("FORMAT_UNKNOWN_ALGORITHM").arg(field).arg(value)
            }
            Self::Malformed(why) => Msg::new("FORMAT_MALFORMED").arg(why),
            Self::NameTooLong => Msg::new("FORMAT_NAME_TOO_LONG"),
            Self::ReservedName => Msg::new("FORMAT_RESERVED_NAME"),
            Self::NotInThisProfile("decoys") => Msg::new("FORMAT_PRIVATE_PROFILE_NO_DECOYS"),
            Self::NotInThisProfile(_) => Msg::new("FORMAT_PRIVATE_PROFILE_NO_INBOX"),
            Self::UnsupportedNodeType => Msg::new("FORMAT_UNSUPPORTED_NODE"),
            Self::RandomUnavailable => Msg::new("RANDOM_UNAVAILABLE"),
            Self::NoRecoveryRecipient => Msg::new("NO_RECOVERY_PHRASE"),
            Self::NoFido2Recipient => Msg::new("FORMAT_NO_FIDO2"),
            Self::NoPlatformRecipient => Msg::new("FORMAT_NO_PLATFORM"),
            Self::NoMemberRecipient => Msg::new("FORMAT_NO_MEMBER"),
            Self::InvalidMemberPublicKey | Self::InvalidInboxPublicKey => {
                Msg::new("FORMAT_INVALID_PUBLIC_KEY")
            }
            Self::TwoFactorRequired => Msg::new("TWO_FACTOR_REQUIRED"),
            Self::NoSuchRecipient => Msg::new("FORMAT_NO_SUCH_RECIPIENT"),
            Self::LastRecipient => Msg::new("FORMAT_LAST_RECIPIENT"),
            Self::RekeyCredentialRequired("fido2") => Msg::new("FORMAT_REKEY_FIDO2"),
            Self::RekeyCredentialRequired(_) => Msg::new("FORMAT_REKEY_PASSWORD"),
        }
    }
}

display_as_msg!(FormatError);

impl std::error::Error for FormatError {}

impl From<cu_crypto::CryptoError> for FormatError {
    fn from(err: cu_crypto::CryptoError) -> Self {
        match err {
            cu_crypto::CryptoError::DecryptFailed | cu_crypto::CryptoError::InvalidParams(_) => {
                Self::DecryptFailed
            }
            cu_crypto::CryptoError::RandomUnavailable => Self::RandomUnavailable,
            cu_crypto::CryptoError::InvalidMnemonic(_) => {
                unreachable!("cu-format never calls the cu-crypto recovery phrase API")
            }
        }
    }
}
