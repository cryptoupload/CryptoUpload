// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::error::CoreError;

static ACCEPT_NEXT_ROLLBACK: Mutex<Option<Instant>> = Mutex::new(None);

const ACCEPT_VALIDITY: Duration = Duration::from_secs(120);

pub fn accept_next_config_rollback() {
    *ACCEPT_NEXT_ROLLBACK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
}

fn take_acceptance() -> bool {
    ACCEPT_NEXT_ROLLBACK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .is_some_and(|armed| armed.elapsed() < ACCEPT_VALIDITY)
}

fn head_path(data_dir: &Path, vault_id: &[u8; 32]) -> PathBuf {
    data_dir
        .join("cryptoupload")
        .join("config-heads")
        .join(data_encoding::HEXLOWER.encode(vault_id))
}

fn read_remembered(path: &Path) -> Result<Option<u64>, CoreError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text.trim().parse().unwrap_or(u64::MAX))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_remembered(path: &Path, seq: u64) -> Result<(), CoreError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    let mut nonce = [0u8; 8];
    cu_crypto::random::fill_random(&mut nonce).map_err(|_| CoreError::RandomUnavailable)?;
    let tmp = parent.join(format!(".{}.tmp", data_encoding::HEXLOWER.encode(&nonce)));
    std::fs::write(&tmp, seq.to_string())?;
    crate::hardening::restrict_to_owner(&tmp, 0o600);
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

pub(crate) fn check_and_remember(
    data_dir: &Path,
    vault_id: &[u8; 32],
    served: u64,
) -> Result<(), CoreError> {
    let accept = take_acceptance();
    check_with(data_dir, vault_id, served, accept)
}

fn check_with(
    data_dir: &Path,
    vault_id: &[u8; 32],
    served: u64,
    accept: bool,
) -> Result<(), CoreError> {
    let path = head_path(data_dir, vault_id);
    match read_remembered(&path)? {
        Some(remembered) if served < remembered && !accept => {
            Err(CoreError::ConfigRolledBack { served, remembered })
        }
        Some(remembered) if served == remembered => Ok(()),
        Some(remembered) if served > remembered => write_remembered(&path, served),
        _ => write_remembered(&path, served),
    }
}

pub(crate) fn remember_written(
    data_dir: &Path,
    vault_id: &[u8; 32],
    written: u64,
) -> Result<(), CoreError> {
    let path = head_path(data_dir, vault_id);
    match read_remembered(&path)? {
        Some(remembered) if remembered >= written => Ok(()),
        _ => write_remembered(&path, written),
    }
}
