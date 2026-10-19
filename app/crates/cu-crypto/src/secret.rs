// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;

use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use crate::error::CryptoError;
use crate::random;

#[derive(Clone)]
pub struct Secret<const N: usize>([u8; N]);

impl<const N: usize> Secret<N> {
    #[must_use]
    pub const fn new(bytes: [u8; N]) -> Self {
        Self(bytes)
    }

    pub fn random() -> Result<Self, CryptoError> {
        let mut bytes = [0u8; N];
        random::fill_random(&mut bytes)?;
        Ok(Self(bytes))
    }

    #[must_use]
    pub const fn expose(&self) -> &[u8; N] {
        &self.0
    }
}

impl<const N: usize> fmt::Debug for Secret<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret<{N}>(***)")
    }
}

impl<const N: usize> PartialEq for Secret<N> {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

impl<const N: usize> Eq for Secret<N> {}

impl<const N: usize> Drop for Secret<N> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<const N: usize> From<[u8; N]> for Secret<N> {
    fn from(bytes: [u8; N]) -> Self {
        Self(bytes)
    }
}
