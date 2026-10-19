// SPDX-License-Identifier: GPL-3.0-or-later

pub const SPEC_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeedId(pub [u8; 4]);

impl SeedId {
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }
}
