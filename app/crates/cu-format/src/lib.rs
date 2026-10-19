// SPDX-License-Identifier: GPL-3.0-or-later
#![warn(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod common;
pub mod config;
pub mod content;
pub mod dirid;
pub mod dirmeta;
pub mod error;
pub mod header;
pub mod inbox;
pub mod keys;
pub mod manifest;
pub mod manifest_v2;
pub mod name;
pub mod padding;
pub mod policy;
pub mod share;

pub use common::{SPEC_VERSION, SeedId};
pub use error::FormatError;
