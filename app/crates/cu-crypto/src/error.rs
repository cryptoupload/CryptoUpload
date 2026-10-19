// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoError {
    DecryptFailed,
    InvalidMnemonic(MnemonicError),
    InvalidParams(&'static str),
    RandomUnavailable,
}

impl Coded for CryptoError {
    fn msg(&self) -> Msg {
        match self {
            Self::DecryptFailed => Msg::new("DECRYPT_FAILED"),
            Self::InvalidMnemonic(e) => e.msg(),
            Self::InvalidParams(why) => Msg::new("CRYPTO_INVALID_PARAMS").arg(why),
            Self::RandomUnavailable => Msg::new("RANDOM_UNAVAILABLE"),
        }
    }
}

display_as_msg!(CryptoError);

impl std::error::Error for CryptoError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MnemonicError {
    WrongWordCount(usize),
    UnknownWord(usize),
    InvalidChecksum,
    Other,
}

impl Coded for MnemonicError {
    fn msg(&self) -> Msg {
        match self {
            Self::WrongWordCount(n) => Msg::new("MNEMONIC_WORD_COUNT").arg(n),
            Self::UnknownWord(i) => Msg::new("MNEMONIC_UNKNOWN_WORD").arg(i + 1),
            Self::InvalidChecksum => Msg::new("MNEMONIC_CHECKSUM"),
            Self::Other => Msg::new("MNEMONIC_INVALID"),
        }
    }
}

display_as_msg!(MnemonicError);

impl From<bip39::Error> for MnemonicError {
    fn from(err: bip39::Error) -> Self {
        match err {
            bip39::Error::BadWordCount(n) => Self::WrongWordCount(n),
            bip39::Error::UnknownWord(i) => Self::UnknownWord(i),
            bip39::Error::InvalidChecksum => Self::InvalidChecksum,
            bip39::Error::BadEntropyBitCount(_) | bip39::Error::AmbiguousLanguages(_) => {
                Self::Other
            }
        }
    }
}
