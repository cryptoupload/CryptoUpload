// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use cu_crypto::kdf::KdfParams;
use cu_crypto::x25519::StaticKeypair;

use crate::error::SecretsError;
use crate::file::write_private;

const MAGIC: &[u8; 6] = b"CUMID1";
const HEADER_LEN: usize = MAGIC.len() + 4 + 4 + 4 + crate::SALT_LEN;
const WRAPPED_KEY_LEN: usize = cu_crypto::keywrap::WRAPPED_KEY_LEN;
const FILE_LEN: usize = HEADER_LEN + WRAPPED_KEY_LEN;

#[must_use]
pub fn identity_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join("member_identity.key")
}

#[must_use]
pub fn exists(path: &Path) -> bool {
    path.exists()
}

pub fn generate_and_save(path: &Path, passphrase: &[u8]) -> Result<StaticKeypair, SecretsError> {
    if path.exists() {
        return Err(SecretsError::IdentityExists);
    }
    let keypair = StaticKeypair::generate()?;
    save(path, &keypair, passphrase)?;
    Ok(keypair)
}

fn save(path: &Path, keypair: &StaticKeypair, passphrase: &[u8]) -> Result<(), SecretsError> {
    let params = KdfParams::sensitive();
    let mut salt = [0u8; crate::SALT_LEN];
    cu_crypto::random::fill_random(&mut salt)?;
    let kek = cu_crypto::kdf::derive_kek(passphrase, &salt, params)?;
    let wrapped = cu_crypto::keywrap::wrap(&kek, &keypair.private_bytes())?;
    debug_assert_eq!(wrapped.len(), WRAPPED_KEY_LEN);

    let mut out = Vec::with_capacity(FILE_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&params.m_cost_kib().to_le_bytes());
    out.extend_from_slice(&params.t_cost().to_le_bytes());
    out.extend_from_slice(&params.p_cost().to_le_bytes());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&wrapped);
    debug_assert_eq!(out.len(), FILE_LEN);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_private(path, &out)?;
    Ok(())
}

#[allow(clippy::indexing_slicing, clippy::expect_used)]
pub fn load(path: &Path, passphrase: &[u8]) -> Result<StaticKeypair, SecretsError> {
    let bytes = std::fs::read(path)?;
    if bytes.len() != FILE_LEN {
        return Err(SecretsError::Malformed(
            "member identity file of unexpected length",
        ));
    }
    if &bytes[0..MAGIC.len()] != MAGIC {
        return Err(SecretsError::Malformed(
            "member identity file: magic bytes missing",
        ));
    }
    let m_cost_kib = u32::from_le_bytes(bytes[6..10].try_into().expect("4-byte slice"));
    let t_cost = u32::from_le_bytes(bytes[10..14].try_into().expect("4-byte slice"));
    let p_cost = u32::from_le_bytes(bytes[14..18].try_into().expect("4-byte slice"));
    let salt = &bytes[18..HEADER_LEN];
    let wrapped = &bytes[HEADER_LEN..];

    let params = KdfParams::from_untrusted(m_cost_kib, t_cost, p_cost)?;
    let kek = cu_crypto::kdf::derive_kek(passphrase, salt, params)?;
    let private_bytes = cu_crypto::keywrap::unwrap(&kek, wrapped)?;
    let private_bytes: [u8; 32] = private_bytes
        .as_slice()
        .try_into()
        .map_err(|_| SecretsError::Malformed("unwrapped private key is not 32 bytes"))?;
    Ok(StaticKeypair::from_private_bytes(private_bytes))
}
