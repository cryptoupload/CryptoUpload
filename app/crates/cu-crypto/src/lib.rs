// SPDX-License-Identifier: GPL-3.0-or-later
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod aead;
pub mod concat_kdf;
pub mod error;
pub mod hash;
pub mod hkdf;
pub mod kdf;
pub mod keywrap;
pub mod mac;
pub mod random;
pub mod secret;
pub mod seed;
pub mod siv;
pub mod slip39;
pub mod x25519;

pub use error::{CryptoError, MnemonicError};
pub use secret::Secret;
