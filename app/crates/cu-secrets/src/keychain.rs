// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::mpsc;
use std::time::Duration;

use crate::error::SecretsError;

const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(3);

fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(KEYCHAIN_TIMEOUT).ok()
}

fn timeout_err() -> SecretsError {
    SecretsError::KeychainTimeout(KEYCHAIN_TIMEOUT.as_secs())
}

pub fn store_password(service: &str, account: &str, password: &str) -> Result<(), SecretsError> {
    let service = service.to_string();
    let account = account.to_string();
    let password = zeroize::Zeroizing::new(password.to_string());
    with_timeout(move || {
        let entry = keyring::Entry::new(&service, &account).map_err(|e| keychain_err(&e))?;
        entry.set_password(&password).map_err(|e| keychain_err(&e))
    })
    .unwrap_or_else(|| Err(timeout_err()))
}

pub fn load_password(service: &str, account: &str) -> Result<Option<String>, SecretsError> {
    let service = service.to_string();
    let account = account.to_string();
    let Some(result) = with_timeout(move || {
        let entry = keyring::Entry::new(&service, &account).ok()?;
        match entry.get_password() {
            Ok(password) => Some(Ok(Some(password))),
            Err(
                keyring::Error::NoEntry
                | keyring::Error::PlatformFailure(_)
                | keyring::Error::NoStorageAccess(_),
            ) => Some(Ok(None)),
            Err(e) => Some(Err(keychain_err(&e))),
        }
    }) else {
        return Ok(None);
    };
    result.unwrap_or(Ok(None))
}

pub fn delete_password(service: &str, account: &str) -> Result<(), SecretsError> {
    let service = service.to_string();
    let account = account.to_string();
    let Some(result) = with_timeout(move || {
        let entry = keyring::Entry::new(&service, &account).ok()?;
        match entry.delete_credential() {
            Ok(())
            | Err(
                keyring::Error::NoEntry
                | keyring::Error::PlatformFailure(_)
                | keyring::Error::NoStorageAccess(_),
            ) => Some(Ok(())),
            Err(e) => Some(Err(keychain_err(&e))),
        }
    }) else {
        return Ok(());
    };
    result.unwrap_or(Ok(()))
}

fn keychain_err(e: &keyring::Error) -> SecretsError {
    SecretsError::Keychain(e.to_string())
}

#[cfg(windows)]
pub fn request_windows_consent(reason: &str) -> Result<bool, SecretsError> {
    use windows::Security::Credentials::UI::{UserConsentVerificationResult, UserConsentVerifier};
    use windows::core::HSTRING;

    let message = HSTRING::from(reason);
    let operation =
        UserConsentVerifier::RequestVerificationAsync(&message).map_err(|e| keychain_err_hr(&e))?;
    let result = operation.get().map_err(|e| keychain_err_hr(&e))?;
    Ok(result == UserConsentVerificationResult::Verified)
}

#[cfg(not(windows))]
pub fn request_windows_consent(_reason: &str) -> Result<bool, SecretsError> {
    Ok(true)
}

#[cfg(windows)]
fn keychain_err_hr(e: &windows::core::Error) -> SecretsError {
    SecretsError::Keychain(e.to_string())
}

pub fn load_password_with_consent(
    service: &str,
    account: &str,
    reason: &str,
) -> Result<Option<String>, SecretsError> {
    if !request_windows_consent(reason)? {
        return Ok(None);
    }
    load_password(service, account)
}
