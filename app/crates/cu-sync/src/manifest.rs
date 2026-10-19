// SPDX-License-Identifier: GPL-3.0-or-later

use cu_format::config::VaultConfig;
use cu_index::{Db, ManifestHeadRow};

use crate::error::SyncError;

pub(crate) struct HeadStores<'a> {
    pub(crate) job: Option<&'a mut Db>,
    pub(crate) shared: Option<&'a mut Db>,
}

pub(crate) fn load_head(
    stores: &HeadStores<'_>,
    vault_id: &[u8; 32],
) -> Result<Option<ManifestHeadRow>, SyncError> {
    let job = match &stores.job {
        Some(j) => j.get_manifest_head(vault_id)?,
        None => None,
    };
    let shared = match &stores.shared {
        Some(h) => h.get_manifest_head(vault_id)?,
        None => None,
    };
    Ok(match (shared, job) {
        (Some(s), Some(j)) => Some(if j.seq > s.seq { j } else { s }),
        (s, j) => s.or(j),
    })
}

pub(crate) fn store_head(stores: &HeadStores<'_>, head: &ManifestHeadRow) -> Result<(), SyncError> {
    if let Some(h) = &stores.shared {
        h.advance_manifest_head(head)?;
    }
    if let Some(j) = &stores.job {
        j.advance_manifest_head(head)?;
    }
    Ok(())
}

pub(crate) struct ManifestKeys {
    pub(crate) vault_id: [u8; 32],
    mac_key: cu_crypto::Secret<32>,
}

impl ManifestKeys {
    pub(crate) fn mac_key(&self) -> &cu_crypto::Secret<32> {
        &self.mac_key
    }
}

impl ManifestKeys {
    pub(crate) fn derive(config: &VaultConfig) -> Result<Self, SyncError> {
        let vault_id = cu_format::keys::root_dir_id(config.initial_seed(), config.kdf_salt())?;
        let mac_key = cu_format::keys::manifest_mac_key(config.initial_seed(), config.kdf_salt())?;
        Ok(Self { vault_id, mac_key })
    }
}
