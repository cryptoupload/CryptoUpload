// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use cu_crypto::siv::{self, NameKey};
use cu_index::FieldCipher;
use data_encoding::BASE64URL_NOPAD;

use crate::error::CoreError;
use crate::vault::Vault;

const DOMAIN: &[u8] = b"org.cryptoupload.localState.v1";

struct StateCipher(NameKey);

impl FieldCipher for StateCipher {
    fn seal(&self, plain: &str) -> Option<String> {
        siv::encrypt_name(&self.0, DOMAIN, plain.as_bytes())
            .ok()
            .map(|sealed| BASE64URL_NOPAD.encode(&sealed))
    }

    fn open(&self, stored: &str) -> Option<String> {
        let sealed = BASE64URL_NOPAD.decode(stored.as_bytes()).ok()?;
        let plain = siv::decrypt_name(&self.0, DOMAIN, &sealed).ok()?;
        String::from_utf8(plain).ok()
    }
}

#[derive(Clone)]
pub struct LocalStateAccess {
    pub(crate) cipher: Option<Arc<dyn FieldCipher>>,
    pub(crate) encrypt: bool,
    pub(crate) log_disabled: bool,
    pub(crate) writable: bool,
}

impl std::fmt::Debug for LocalStateAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalStateAccess")
            .field("has_key", &self.cipher.is_some())
            .field("encrypt", &self.encrypt)
            .field("log_disabled", &self.log_disabled)
            .field("writable", &self.writable)
            .finish()
    }
}

impl LocalStateAccess {
    #[must_use]
    pub fn locked() -> Self {
        Self {
            cipher: None,
            encrypt: false,
            log_disabled: false,
            writable: false,
        }
    }

    #[must_use]
    pub fn plain() -> Self {
        Self {
            cipher: None,
            encrypt: false,
            log_disabled: false,
            writable: true,
        }
    }

    #[must_use]
    pub fn cipher(&self) -> Option<Arc<dyn FieldCipher>> {
        self.cipher.clone()
    }
}

impl Vault {
    pub fn local_state(&self) -> Result<LocalStateAccess, CoreError> {
        let config = self.config();
        let key = cu_format::keys::local_state_key(config.initial_seed(), config.kdf_salt())?;
        let policy = self.effective_policy();
        Ok(LocalStateAccess {
            cipher: Some(Arc::new(StateCipher(key))),
            encrypt: policy.encrypt_local_state,
            log_disabled: policy.disable_activity_log,
            writable: true,
        })
    }
}
