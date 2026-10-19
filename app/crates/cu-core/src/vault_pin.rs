// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};

use crate::error::CoreError;

fn pin_path(data_dir: &Path, root: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut hasher = cu_crypto::hash::Hasher::new();
    hasher.update(canonical.as_os_str().as_encoded_bytes());
    data_dir
        .join("cryptoupload")
        .join("member-vaults")
        .join(data_encoding::HEXLOWER.encode(&hasher.finalize()))
}

pub(crate) fn check_or_remember(
    data_dir: &Path,
    root: &Path,
    vault_id: &[u8; 32],
) -> Result<(), CoreError> {
    let path = pin_path(data_dir, root);
    let expected = data_encoding::HEXLOWER.encode(vault_id);
    match std::fs::read_to_string(&path) {
        Ok(known) if known.trim() == expected => Ok(()),
        Ok(_) => Err(CoreError::VaultIdentityChanged),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, expected)?;
            crate::hardening::restrict_to_owner(&path, 0o600);
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

pub fn forget_member_vault_identity(data_dir: &Path, root: &Path) -> Result<(), CoreError> {
    match std::fs::remove_file(pin_path(data_dir, root)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
