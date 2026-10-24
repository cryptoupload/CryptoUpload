// SPDX-License-Identifier: GPL-3.0-or-later

use argon2::{Algorithm, Argon2, Params, Version};

use crate::error::CryptoError;
use crate::secret::Secret;

pub const KEK_LEN: usize = 32;

pub type Kek = Secret<KEK_LEN>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    m_cost_kib: u32,
    t_cost: u32,
    p_cost: u32,
}

impl KdfParams {
    #[must_use]
    pub const fn sensitive() -> Self {
        Self {
            m_cost_kib: 256 * 1024,
            t_cost: 4,
            p_cost: 4,
        }
    }

    #[must_use]
    pub const fn new(m_cost_kib: u32, t_cost: u32, p_cost: u32) -> Self {
        Self {
            m_cost_kib,
            t_cost,
            p_cost,
        }
    }

    pub const MAX_UNTRUSTED_M_COST_KIB: u32 = 2 * 1024 * 1024;
    pub const MAX_UNTRUSTED_T_COST: u32 = 64;
    pub const MAX_UNTRUSTED_P_COST: u32 = 16;

    pub fn from_untrusted(m_cost_kib: u32, t_cost: u32, p_cost: u32) -> Result<Self, CryptoError> {
        if m_cost_kib > Self::MAX_UNTRUSTED_M_COST_KIB {
            return Err(CryptoError::InvalidParams(
                "argon2: m_cost above the ceiling allowed for an untrusted file",
            ));
        }
        if t_cost > Self::MAX_UNTRUSTED_T_COST {
            return Err(CryptoError::InvalidParams(
                "argon2: t_cost above the ceiling allowed for an untrusted file",
            ));
        }
        if p_cost > Self::MAX_UNTRUSTED_P_COST {
            return Err(CryptoError::InvalidParams(
                "argon2: p_cost above the ceiling allowed for an untrusted file",
            ));
        }
        Ok(Self::new(m_cost_kib, t_cost, p_cost))
    }

    #[must_use]
    pub const fn m_cost_kib(&self) -> u32 {
        self.m_cost_kib
    }

    #[must_use]
    pub const fn t_cost(&self) -> u32 {
        self.t_cost
    }

    #[must_use]
    pub const fn p_cost(&self) -> u32 {
        self.p_cost
    }
}

pub fn derive_kek(password: &[u8], salt: &[u8], params: KdfParams) -> Result<Kek, CryptoError> {
    let argon2_params = Params::new(
        params.m_cost_kib,
        params.t_cost,
        params.p_cost,
        Some(KEK_LEN),
    )
    .map_err(|_| CryptoError::InvalidParams("argon2: parameters out of range"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);

    let mut out = [0u8; KEK_LEN];
    argon2
        .hash_password_into(password, salt, &mut out)
        .map_err(|_| CryptoError::InvalidParams("argon2: password or salt out of range"))?;
    Ok(Secret::new(out))
}
