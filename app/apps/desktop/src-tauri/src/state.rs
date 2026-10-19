// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::Mutex;

use crate::settings::UiSettings;

#[derive(Clone)]
pub enum VaultLocation {
    Local(PathBuf),
    Remote(String),
}

pub struct OpenVault {
    pub vault: cu_core::Vault,
    pub location: VaultLocation,
}

#[cfg(any(target_os = "linux", windows))]
pub struct MountedVault {
    pub handle: cu_core::MountHandle,
    pub mountpoint: PathBuf,
    pub read_only: bool,
}

pub struct UnlockedRemote {
    pub config: cu_core::RemoteConfig,
    pub password: String,
}

impl Drop for UnlockedRemote {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.password);
    }
}

pub struct AppState {
    pub vault: Mutex<Option<OpenVault>>,
    pub vault_generation: AtomicU64,
    pub unlocked_remotes: Mutex<HashMap<String, UnlockedRemote>>,
    pub data_dir: PathBuf,
    pub settings: Mutex<UiSettings>,
    #[cfg(any(target_os = "linux", windows))]
    pub mount: Mutex<Option<MountedVault>>,
    pub running_jobs: std::sync::Mutex<HashMap<String, cu_core::CancelToken>>,
    pub find_cancel: std::sync::Mutex<Option<cu_core::CancelToken>>,
    pub remote_acknowledged: std::sync::Mutex<std::collections::BTreeSet<String>>,
    pub lock_notice: std::sync::Mutex<Option<serde_json::Value>>,
    pub denied_programs: std::sync::Mutex<std::collections::BTreeSet<String>>,
    pub access_requests: std::sync::Mutex<HashMap<u64, tokio::sync::oneshot::Sender<bool>>>,
    pub next_access_request: AtomicU64,
    pub local_state: std::sync::Mutex<Option<(cu_core::VaultRef, cu_core::LocalStateAccess)>>,
    pub memory_lock: std::sync::Mutex<Option<cu_core::memlock::MemoryLockReport>>,
    pub ask_denied: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

impl AppState {
    #[must_use]
    pub fn new() -> Self {
        let data_dir = cu_core::data_dir()
            .expect("cannot compute the app data directory: environment variable missing");
        Self::with_data_dir(data_dir)
    }

    #[must_use]
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        let settings = crate::settings::load(&data_dir);
        Self {
            vault: Mutex::new(None),
            vault_generation: AtomicU64::new(0),
            unlocked_remotes: Mutex::new(HashMap::new()),
            data_dir,
            settings: Mutex::new(settings),
            running_jobs: std::sync::Mutex::new(HashMap::new()),
            find_cancel: std::sync::Mutex::new(None),
            remote_acknowledged: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            lock_notice: std::sync::Mutex::new(None),
            denied_programs: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            access_requests: std::sync::Mutex::new(HashMap::new()),
            next_access_request: AtomicU64::new(1),
            local_state: std::sync::Mutex::new(None),
            memory_lock: std::sync::Mutex::new(None),
            ask_denied: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            #[cfg(any(target_os = "linux", windows))]
            mount: Mutex::new(None),
        }
    }

    pub async fn take_vault(&self) -> Option<(OpenVault, u64)> {
        take_slot(&self.vault, &self.vault_generation).await
    }

    pub async fn restore_vault(&self, open_vault: OpenVault, generation: u64) -> bool {
        restore_slot(&self.vault, &self.vault_generation, generation, open_vault).await
    }

    pub async fn store_vault(&self, open_vault: OpenVault) {
        self.remember_local_state(&open_vault);
        let mut guard = self.vault.lock().await;
        self.vault_generation.fetch_add(1, Ordering::SeqCst);
        *guard = Some(open_vault);
    }

    pub async fn clear_vault(&self) {
        clear_slot(&self.vault, &self.vault_generation).await;
        *self
            .local_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *self
            .memory_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    pub fn lock_vault_memory(&self, vault: &mut cu_core::Vault) {
        let report = vault.lock_memory();
        *self
            .memory_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(report);
    }

    pub fn remember_local_state(&self, open_vault: &OpenVault) {
        let vault_ref = match &open_vault.location {
            VaultLocation::Local(path) => cu_core::VaultRef::Local(path.clone()),
            VaultLocation::Remote(name) => cu_core::VaultRef::Remote(name.clone()),
        };
        *self
            .local_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = open_vault
            .vault
            .local_state()
            .ok()
            .map(|access| (vault_ref, access));
    }

    #[must_use]
    pub fn activity_log_for(&self, vault: &cu_core::VaultRef) -> cu_core::activity::ActivityLog {
        cu_core::activity::ActivityLog::for_vault(
            &self.data_dir,
            vault,
            &self.local_state_for(vault),
        )
    }

    #[must_use]
    pub fn local_state_for(&self, vault: &cu_core::VaultRef) -> cu_core::LocalStateAccess {
        self.local_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .filter(|(open, _)| cu_core::overview::same_vault(open, vault))
            .map_or_else(cu_core::LocalStateAccess::locked, |(_, access)| {
                access.clone()
            })
    }

    #[must_use]
    pub fn secrets_file_path(&self) -> PathBuf {
        cu_core::secrets_file_path(&self.data_dir)
    }
}

async fn take_slot<T>(slot: &Mutex<Option<T>>, generation: &AtomicU64) -> Option<(T, u64)> {
    let mut guard = slot.lock().await;
    let value = guard.take()?;
    Some((value, generation.load(Ordering::SeqCst)))
}

async fn restore_slot<T>(
    slot: &Mutex<Option<T>>,
    generation: &AtomicU64,
    taken_at: u64,
    value: T,
) -> bool {
    let mut guard = slot.lock().await;
    if generation.load(Ordering::SeqCst) != taken_at || guard.is_some() {
        drop(guard);
        drop(value);
        return false;
    }
    *guard = Some(value);
    true
}

async fn clear_slot<T>(slot: &Mutex<Option<T>>, generation: &AtomicU64) {
    let mut guard = slot.lock().await;
    generation.fetch_add(1, Ordering::SeqCst);
    *guard = None;
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
