// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

use bip39::{Language, Mnemonic};

use crate::error::{CryptoError, MnemonicError};
use crate::secret::Secret;

pub const ENTROPY_LEN: usize = 32;

const WORD_COUNT: usize = 24;

pub struct RecoveryPhrase(Mnemonic);

impl RecoveryPhrase {
    pub fn generate() -> Result<Self, CryptoError> {
        let entropy = Secret::<ENTROPY_LEN>::random()?;
        Ok(Self::from_entropy(&entropy))
    }

    #[must_use]
    pub fn from_entropy(entropy: &Secret<ENTROPY_LEN>) -> Self {
        #[allow(clippy::expect_used)]
        let mnemonic = Mnemonic::from_entropy_in(Language::English, entropy.expose())
            .expect("32 bytes are always valid BIP39 entropy");
        Self(mnemonic)
    }

    pub fn parse(words: &str) -> Result<Self, CryptoError> {
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, words)
            .map_err(|e| CryptoError::InvalidMnemonic(MnemonicError::from(e)))?;
        if mnemonic.word_count() != WORD_COUNT {
            return Err(CryptoError::InvalidMnemonic(MnemonicError::WrongWordCount(
                mnemonic.word_count(),
            )));
        }
        Ok(Self(mnemonic))
    }

    pub fn words(&self) -> impl Iterator<Item = &'static str> + Clone + '_ {
        self.0.words()
    }

    #[must_use]
    pub fn entropy(&self) -> Secret<ENTROPY_LEN> {
        let (bytes, len) = self.0.to_entropy_array();
        debug_assert_eq!(
            len, ENTROPY_LEN,
            "a 24-word recovery phrase always has 32 bytes of entropy"
        );
        let mut out = [0u8; ENTROPY_LEN];
        out.copy_from_slice(&bytes[..ENTROPY_LEN]);
        Secret::new(out)
    }
}

impl fmt::Debug for RecoveryPhrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RecoveryPhrase(<redacted>, {} words)",
            self.0.word_count()
        )
    }
}
