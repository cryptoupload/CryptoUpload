// SPDX-License-Identifier: GPL-3.0-or-later

use async_trait::async_trait;
use cu_crypto::aead::NONCE_LEN;
use cu_format::manifest_v2::{EntryKey, Op};

use crate::VfsError;

#[async_trait]
pub trait NamespaceObserver: Send + Sync {
    async fn begin(&self, ops: &[Op]) -> Result<(), VfsError>;

    async fn end(&self, ops: &[Op], committed: bool);

    async fn check_file(&self, _key: &EntryKey, _served: &[u8; NONCE_LEN]) -> Result<(), VfsError> {
        Ok(())
    }

    async fn check_dir(&self, _key: &EntryKey, _served: &[u8; 32]) -> Result<(), VfsError> {
        Ok(())
    }
}
