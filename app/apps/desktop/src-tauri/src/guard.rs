// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

pub const FORCE_LOCKED_EVENT: &str = "vault-force-locked";

pub const FLUSH_BUDGET: Duration = Duration::from_secs(20);

pub const FLUSH_BUDGET_BEFORE_SLEEP: Duration = Duration::from_secs(2);

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LockReason {
    Idle,
    ScreenLocked,
    Suspend,
    SessionInactive,
    RemoteSession,
    Tripwire,
    Panic,
    WindowClosed,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ForceLockReport {
    pub locked: bool,
    pub reason: LockReason,
    pub was_mounted: bool,
    pub unpublished: usize,
    pub drive_still_readable: bool,
    pub cancelled_jobs: usize,
    pub detail: Option<String>,
}

pub async fn force_lock(
    state: &AppState,
    reason: LockReason,
    flush_budget: Duration,
) -> ForceLockReport {
    let cancelled_jobs = {
        let running = state
            .running_jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for token in running.values() {
            token.cancel();
        }
        running.len()
    };

    let mut report = ForceLockReport {
        locked: false,
        reason,
        was_mounted: false,
        unpublished: 0,
        drive_still_readable: false,
        cancelled_jobs,
        detail: None,
    };

    #[cfg(any(target_os = "linux", windows))]
    {
        let mut mount_guard = state.mount.lock().await;
        if let Some(mounted) = mount_guard.take() {
            report.was_mounted = true;
            let flusher = mounted.handle.flusher();
            report.unpublished = match tokio::time::timeout(flush_budget, flusher.flush()).await {
                Ok(failed) => failed,
                Err(_) => flusher.pending_files(),
            };
            report.drive_still_readable = !matches!(
                mounted.handle.unmount_forced(),
                Ok(cu_core::ForcedUnmount::Severed)
            );
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = flush_budget;

    report.locked = state.vault.lock().await.is_some() || cancelled_jobs > 0;
    state.clear_vault().await;
    state.unlocked_remotes.lock().await.clear();
    report
}

pub async fn force_lock_and_notify(app: &AppHandle, reason: LockReason, flush_budget: Duration) {
    let state = app.state::<AppState>();
    let report = force_lock(&state, reason, flush_budget).await;
    if report.locked {
        let _ = app.emit(FORCE_LOCKED_EVENT, report);
    }
}

pub const ACCESS_REQUEST_EVENT: &str = "drive-access-request";

const MAX_DENIED_PROGRAMS: usize = 50;

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AccessRequest {
    id: u64,
    program: String,
}

#[cfg(any(target_os = "linux", windows))]
pub struct DriveSentry {
    app: AppHandle,
}

#[cfg(any(target_os = "linux", windows))]
impl DriveSentry {
    #[must_use]
    pub fn new(app: &AppHandle) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { app: app.clone() })
    }
}

#[cfg(any(target_os = "linux", windows))]
impl cu_core::SentryEvents for DriveSentry {
    fn denied(&self, program: &str) {
        let state = self.app.state::<AppState>();
        let mut denied = state
            .denied_programs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if denied.len() < MAX_DENIED_PROGRAMS {
            denied.insert(program.to_string());
        }
    }

    fn ask<'a>(&'a self, program: &'a str) -> cu_core::AskFuture<'a> {
        Box::pin(async move {
            let state = self.app.state::<AppState>();
            let already_denied = state
                .ask_denied
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(program);
            if already_denied {
                return false;
            }
            let id = state
                .next_access_request
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let (tx, rx) = tokio::sync::oneshot::channel();
            state
                .access_requests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id, tx);
            let request = AccessRequest {
                id,
                program: program.to_string(),
            };
            if self.app.emit(ACCESS_REQUEST_EVENT, request).is_err() {
                return false;
            }
            let answer = rx.await;
            if answer == Ok(false) {
                state
                    .ask_denied
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(program.to_string());
            }
            let answer = answer.unwrap_or(false);
            state
                .access_requests
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            answer
        })
    }

    fn tripwire(&self, trip: cu_core::Tripwire) {
        let program = match trip {
            cu_core::Tripwire::Decoy { program } | cu_core::Tripwire::MassRead { program, .. } => {
                program
            }
        };
        let handle = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let state = handle.state::<AppState>();
            let mut report =
                force_lock(&state, LockReason::Tripwire, FLUSH_BUDGET_BEFORE_SLEEP).await;
            report.detail = program;
            if report.locked {
                let _ = handle.emit(FORCE_LOCKED_EVENT, report);
            }
        });
    }
}

pub async fn remote_evidence() -> std::collections::BTreeSet<String> {
    tauri::async_runtime::spawn_blocking(|| {
        cu_core::sentinel::remote_control_evidence()
            .into_iter()
            .map(|evidence| evidence.0)
            .collect()
    })
    .await
    .unwrap_or_default()
}

const REMOTE_CHECK_INTERVAL: Duration = Duration::from_secs(5);

pub fn start_remote_watcher(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(REMOTE_CHECK_INTERVAL).await;
            let state = handle.state::<AppState>();
            let wanted = state
                .vault
                .lock()
                .await
                .as_ref()
                .is_some_and(|open| open.vault.effective_policy().block_remote_sessions);
            if !wanted {
                continue;
            }
            let evidence = remote_evidence().await;
            let is_new = {
                let acknowledged = state
                    .remote_acknowledged
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                evidence.iter().any(|e| !acknowledged.contains(e))
            };
            if is_new {
                force_lock_and_notify(&handle, LockReason::RemoteSession, FLUSH_BUDGET).await;
            }
        }
    });
}

async fn locks_on_session_events(state: &AppState) -> bool {
    state
        .vault
        .lock()
        .await
        .as_ref()
        .is_some_and(|open| open.vault.effective_policy().lock_on_session_events)
}

pub fn start_session_watcher(app: &AppHandle) {
    use cu_core::sentinel::{SessionEvent, watch_session};

    let handle = app.clone();
    let result = watch_session(move |event| {
        let (reason, budget) = match event {
            SessionEvent::Suspending => (LockReason::Suspend, FLUSH_BUDGET_BEFORE_SLEEP),
            SessionEvent::ScreenLocked => (LockReason::ScreenLocked, FLUSH_BUDGET),
            SessionEvent::SessionInactive => (LockReason::SessionInactive, FLUSH_BUDGET),
            SessionEvent::Resumed => return,
        };
        tauri::async_runtime::block_on(async {
            let state = handle.state::<AppState>();
            if locks_on_session_events(&state).await {
                force_lock_and_notify(&handle, reason, budget).await;
            }
        });
    });
    if let Ok(watcher) = result {
        app.manage(watcher);
    }
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PanicReport {
    pub lock: ForceLockReport,
    pub revoked_jobs: usize,
    pub forgotten_quick_unlocks: usize,
}

pub async fn panic(state: &AppState) -> PanicReport {
    let lock = force_lock(state, LockReason::Panic, FLUSH_BUDGET_BEFORE_SLEEP).await;

    let recent: Vec<String> = state.settings.lock().await.recent_vaults.clone();
    let mut forgotten_quick_unlocks = 0;
    for location in recent.iter().filter(|l| !l.starts_with("remote:")) {
        let root = std::path::PathBuf::from(location);
        if matches!(
            cu_core::Vault::forget_platform_unlock_key(&root).await,
            Ok(true)
        ) {
            forgotten_quick_unlocks += 1;
        }
    }

    let jobs_path = cu_core::default_jobs_path(&state.data_dir);
    let revoked_jobs = if jobs_path.exists() {
        cu_core::revoke_auto_unlock(&jobs_path, |_| true).map_or(0, |jobs| jobs.len())
    } else {
        0
    };

    {
        let mut settings = state.settings.lock().await;
        settings.recent_vaults.clear();
        let _ = crate::settings::save(&state.data_dir, &settings);
    }
    cu_core::activity::ActivityLog::clear_all(&state.data_dir);
    cu_core::attempts::clear_all_failed_unlocks(&state.data_dir);

    PanicReport {
        lock,
        revoked_jobs,
        forgotten_quick_unlocks,
    }
}

pub const PANIC_EVENT: &str = "vault-panic";

pub fn panic_and_notify(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = handle.state::<AppState>();
        let report = panic(&state).await;
        let _ = handle.emit(PANIC_EVENT, report);
    });
}

pub const PANIC_ARG: &str = "--panic";

pub fn apply_capture_protection(app: &AppHandle, hidden: bool) {
    for window in app.webview_windows().into_values() {
        let _ = window.set_content_protected(hidden);
    }
}

pub fn set_panic_shortcut(app: &AppHandle, shortcut: Option<&str>) -> Result<(), cu_msg::Msg> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

    let manager = app.global_shortcut();
    let _ = manager.unregister_all();
    let Some(text) = shortcut.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(());
    };
    let parsed: Shortcut = text
        .parse()
        .map_err(|_| cu_msg::Msg::new("PANIC_SHORTCUT_INVALID").arg(text))?;
    let handle = app.clone();
    manager
        .on_shortcut(parsed, move |_app, _shortcut, event| {
            if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                panic_and_notify(&handle);
            }
        })
        .map_err(|_| cu_msg::Msg::new("PANIC_SHORTCUT_REFUSED"))
}
