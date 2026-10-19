// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use cu_format::manifest::Writer;
use cu_index::Db;
use cu_sync::registry::Registrar;

use crate::error::CoreError;
use crate::vault::Vault;

static GLOBAL_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegistryVerifyOptions {
    pub register: bool,
    pub seal: bool,
}

#[derive(Debug, Clone)]
pub struct RegistryVerification {
    pub report: cu_sync::registry_verify::RegistryReport,
    pub registered: u64,
    pub confirmed: u64,
    pub sealed: bool,
}

#[must_use]
pub fn vault_head_db_path(data_dir: &Path, vault_id: &[u8; 32]) -> PathBuf {
    data_dir
        .join("cryptoupload")
        .join("heads")
        .join(format!("{}.db", data_encoding::HEXLOWER.encode(vault_id)))
}

pub fn enable_vault_registry(data_dir: &Path) -> bool {
    let _ = crate::network::apply_saved_proxy(data_dir);
    GLOBAL_DATA_DIR.set(data_dir.to_path_buf()).is_ok()
}

pub(crate) fn global_data_dir() -> Option<&'static Path> {
    GLOBAL_DATA_DIR.get().map(PathBuf::as_path)
}

pub(crate) fn open_head_db(vault: &Vault, data_dir: &Path) -> Result<Db, CoreError> {
    let config = vault.config();
    let vault_id = cu_format::keys::root_dir_id(config.initial_seed(), config.kdf_salt())?;
    let path = vault_head_db_path(data_dir, &vault_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(Db::open(&path)?)
}

impl Vault {
    pub fn with_registry(mut self, data_dir: &Path) -> Result<Self, CoreError> {
        if self.registry.is_some() {
            return Ok(self);
        }
        let config = self.config();
        let vault_id = cu_format::keys::root_dir_id(config.initial_seed(), config.kdf_salt())?;
        crate::config_head::check_and_remember(data_dir, &vault_id, config.config_seq())?;
        let identity = crate::device::current(data_dir)?;
        let heads = open_head_db(&self, data_dir)?;
        let registrar = Registrar::new(
            self.config(),
            Writer {
                device_id: identity.device_id,
                label: identity.label,
            },
            heads,
        )?;
        let fs = Arc::get_mut(&mut self.fs)
            .ok_or_else(|| CoreError::IntegrityFailure(Msg::new("REGISTRY_ENABLE_FIRST")))?;
        fs.set_observer(registrar.clone());
        registrar.bind(Arc::downgrade(&self.fs));
        self.registry = Some(registrar);
        Ok(self)
    }

    pub(crate) fn with_global_registry(self) -> Result<Self, CoreError> {
        match GLOBAL_DATA_DIR.get() {
            Some(dir) => self.with_registry(dir),
            None => Ok(self),
        }
    }

    pub async fn flush_registry(&self) {
        if let Some(registry) = &self.registry {
            let _ = registry.flush().await;
        }
    }

    pub(crate) async fn settled<T>(&self, result: Result<T, CoreError>) -> Result<T, CoreError> {
        self.flush_registry().await;
        result
    }

    pub async fn reset_registry(&self) -> Result<(), CoreError> {
        let registrar = self.registrar().ok_or(CoreError::RegistryDisabled)?;
        registrar.reset().await.map_err(CoreError::IntegrityFailure)
    }

    pub async fn registry_snapshot(
        &self,
    ) -> Result<Option<cu_sync::registry::RegistrySnapshot>, CoreError> {
        Ok(cu_sync::registry::read_snapshot(self.fs(), self.config()).await?)
    }

    pub async fn verify_registry(
        &self,
        options: RegistryVerifyOptions,
    ) -> Result<RegistryVerification, CoreError> {
        let registrar = self.registrar().ok_or(CoreError::RegistryDisabled)?;
        let register = options.register || options.seal;
        self.flush_registry().await;
        let root =
            cu_format::keys::root_dir_id(self.config().initial_seed(), self.config().kdf_salt())?;
        let (state, sealed_before) = match self.registry_snapshot().await? {
            Some(snap) => {
                let sealed = snap.state.coverage;
                (snap.state, sealed)
            }
            None => (cu_format::manifest_v2::State::default(), false),
        };
        let report =
            cu_sync::registry_verify::verify_registry(self.fs(), self.config(), &state, root)
                .await?;

        let mut registered = 0;
        let mut confirmed = 0;
        if register {
            let entries: Vec<_> = report
                .unregistered
                .iter()
                .map(|(_, key, node)| (key.clone(), node.clone()))
                .collect();
            registered = entries.len() as u64;
            confirmed = report.confirmable.len() as u64;
            registrar
                .register(entries, report.confirmable.clone())
                .await
                .map_err(CoreError::IntegrityFailure)?;
        }
        let mut sealed = sealed_before;
        if options.seal && !sealed && report.is_clean() {
            registrar
                .seal()
                .await
                .map_err(CoreError::IntegrityFailure)?;
            sealed = true;
        }
        Ok(RegistryVerification {
            report,
            registered,
            confirmed,
            sealed,
        })
    }

    pub async fn header_nonce(&self, vault_path: &str) -> Result<[u8; 12], CoreError> {
        Ok(self
            .fs()
            .read_header_nonce(self.config(), vault_path)
            .await?)
    }

    pub(crate) fn registrar(&self) -> Option<Arc<Registrar>> {
        self.registry.clone()
    }
}
