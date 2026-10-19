// SPDX-License-Identifier: GPL-3.0-or-later

use x25519_dalek::{PublicKey, StaticSecret};

use crate::error::CryptoError;
use crate::secret::Secret;

pub const KEY_LEN: usize = 32;

pub struct StaticKeypair {
    secret: StaticSecret,
    public: [u8; KEY_LEN],
}

impl StaticKeypair {
    pub fn generate() -> Result<Self, CryptoError> {
        let bytes = Secret::<KEY_LEN>::random()?;
        Ok(Self::from_private_bytes(*bytes.expose()))
    }

    #[must_use]
    pub fn from_private_bytes(bytes: [u8; KEY_LEN]) -> Self {
        let secret = StaticSecret::from(bytes);
        let public = PublicKey::from(&secret).to_bytes();
        Self { secret, public }
    }

    #[must_use]
    pub const fn public_bytes(&self) -> [u8; KEY_LEN] {
        self.public
    }

    #[must_use]
    pub fn private_bytes(&self) -> [u8; KEY_LEN] {
        self.secret.to_bytes()
    }

    pub fn diffie_hellman(
        &self,
        their_public: &[u8; KEY_LEN],
    ) -> Result<Secret<KEY_LEN>, CryptoError> {
        let their = PublicKey::from(*their_public);
        let shared = self.secret.diffie_hellman(&their);
        if !shared.was_contributory() {
            return Err(CryptoError::DecryptFailed);
        }
        Ok(Secret::new(shared.to_bytes()))
    }
}

pub fn ephemeral_diffie_hellman(
    their_public: &[u8; KEY_LEN],
) -> Result<([u8; KEY_LEN], Secret<KEY_LEN>), CryptoError> {
    let ephemeral_bytes = Secret::<KEY_LEN>::random()?;
    let ephemeral = StaticKeypair::from_private_bytes(*ephemeral_bytes.expose());
    let z = ephemeral.diffie_hellman(their_public)?;
    Ok((ephemeral.public_bytes(), z))
}
