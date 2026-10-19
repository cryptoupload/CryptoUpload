// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::{Arc, Mutex};

use russh::keys::HashAlg;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyInfo {
    pub key_type: String,
    pub fingerprint: String,
}

pub(crate) struct Handler {
    pub(crate) observed: Arc<Mutex<Option<HostKeyInfo>>>,
}

impl russh::client::Handler for Handler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        let info = HostKeyInfo {
            key_type: server_public_key.algorithm().to_string(),
            fingerprint: server_public_key.fingerprint(HashAlg::Sha256).to_string(),
        };
        *self.observed.lock().expect("mutex never poisoned") = Some(info);
        std::future::ready(Ok(true))
    }
}

pub async fn probe_host_key(
    host: &str,
    port: u16,
) -> Result<HostKeyInfo, cu_backend::BackendError> {
    let observed = Arc::new(Mutex::new(None));
    let handler = Handler {
        observed: Arc::clone(&observed),
    };
    let config = crate::config::client_config();
    let handle = tokio::time::timeout(
        crate::config::CONNECT_TIMEOUT,
        crate::config::connect(config, host, port, handler),
    )
    .await
    .map_err(|_| cu_backend::BackendError::Transient("sftp: connect timeout".to_string()))??;
    drop(handle);
    observed
        .lock()
        .expect("mutex never poisoned")
        .clone()
        .ok_or_else(|| {
            cu_backend::BackendError::Other(
                "sftp: handshake completed without an observed host key".to_string(),
            )
        })
}
