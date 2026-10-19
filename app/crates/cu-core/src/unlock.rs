// SPDX-License-Identifier: GPL-3.0-or-later

use crate::keychain;

const KEYCHAIN_SERVICE: &str = "cryptoupload-job-vault";

#[must_use]
pub fn try_unlock_vault_password(job_name: &str) -> Option<String> {
    keychain::load_password(KEYCHAIN_SERVICE, job_name)
        .ok()
        .flatten()
}

pub fn store_vault_password_in_keychain(
    job_name: &str,
    password: &str,
) -> Result<(), cu_secrets::SecretsError> {
    keychain::store_password(KEYCHAIN_SERVICE, job_name, password)
}

pub fn forget_vault_password_in_keychain(job_name: &str) -> Result<(), cu_secrets::SecretsError> {
    keychain::delete_password(KEYCHAIN_SERVICE, job_name)
}

pub const SECRETS_KEYCHAIN_SERVICE: &str = "cryptoupload";

#[must_use]
pub fn secrets_keychain_account(remote_name: &str) -> String {
    format!("remotes-secrets-password/{remote_name}")
}

#[must_use]
pub fn try_unlock_remote_password(remote_name: &str) -> Option<String> {
    keychain::load_password(
        SECRETS_KEYCHAIN_SERVICE,
        &secrets_keychain_account(remote_name),
    )
    .ok()
    .flatten()
}

pub fn store_remote_password_in_keychain(
    remote_name: &str,
    password: &str,
) -> Result<(), cu_secrets::SecretsError> {
    keychain::store_password(
        SECRETS_KEYCHAIN_SERVICE,
        &secrets_keychain_account(remote_name),
        password,
    )
}

pub fn forget_remote_password_in_keychain(
    remote_name: &str,
) -> Result<(), cu_secrets::SecretsError> {
    keychain::delete_password(
        SECRETS_KEYCHAIN_SERVICE,
        &secrets_keychain_account(remote_name),
    )
}
