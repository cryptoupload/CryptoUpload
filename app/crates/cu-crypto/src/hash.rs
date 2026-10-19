// SPDX-License-Identifier: GPL-3.0-or-later

#[derive(Debug, Default, Clone)]
pub struct Hasher(blake3::Hasher);

impl Hasher {
    #[must_use]
    pub fn new() -> Self {
        Self(blake3::Hasher::new())
    }

    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.0.update(data);
        self
    }

    #[must_use]
    pub fn finalize(&self) -> [u8; 32] {
        *self.0.finalize().as_bytes()
    }
}
