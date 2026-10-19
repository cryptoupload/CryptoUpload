// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::PathBuf;

use tauri::State;
use tauri::ipc::Channel;
use zeroize::Zeroize;

use crate::error::UiError;
use crate::state::{AppState, OpenVault, VaultLocation};

pub(crate) fn store(state: &AppState) -> cu_core::JobStore {
    cu_core::JobStore::new(cu_core::default_jobs_path(&state.data_dir))
}

pub(crate) fn vault_ref_matches(vault_ref: &cu_core::VaultRef, location: &VaultLocation) -> bool {
    match (vault_ref, location) {
        (cu_core::VaultRef::Local(root), VaultLocation::Local(open_root)) => {
            let a = std::fs::canonicalize(root);
            let b = std::fs::canonicalize(open_root);
            matches!((a, b), (Ok(a), Ok(b)) if a == b)
        }
        (cu_core::VaultRef::Remote(name), VaultLocation::Remote(open_name)) => name == open_name,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimingDto {
    shuffle_uploads: bool,
    random_delay_max_secs: u32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDto {
    name: String,
    enabled: bool,
    local_root: String,
    vault_path: String,
    interval_secs: u64,
    keep_versions: usize,
    keep_runs: u32,
    auto_unlock: bool,
    matches_open_vault: bool,
    excludes: Vec<String>,
    schedule_windows: Vec<ScheduleWindowDto>,
    pre_hook: Option<HookSpecDto>,
    post_hook: Option<HookSpecDto>,
    anomaly_guard: AnomalyGuardDto,
    timing: TimingDto,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyGuardDto {
    enabled: bool,
    mass_modify_abs: u64,
    mass_modify_multiplier: u64,
    mass_delete_abs: u64,
    mass_delete_multiplier: u64,
    rename_min_count: usize,
    rename_uniform_fraction_pct: u8,
    min_baseline_runs: usize,
}

impl From<cu_core::AnomalyGuard> for AnomalyGuardDto {
    fn from(guard: cu_core::AnomalyGuard) -> Self {
        let (enabled, t) = match guard {
            cu_core::AnomalyGuard::Enabled(t) => (true, t),
            cu_core::AnomalyGuard::Disabled => (false, cu_core::AnomalyThresholds::default()),
        };
        Self {
            enabled,
            mass_modify_abs: t.mass_modify_abs,
            mass_modify_multiplier: t.mass_modify_multiplier,
            mass_delete_abs: t.mass_delete_abs,
            mass_delete_multiplier: t.mass_delete_multiplier,
            rename_min_count: t.rename_min_count,
            rename_uniform_fraction_pct: t.rename_uniform_fraction_pct,
            min_baseline_runs: t.min_baseline_runs,
        }
    }
}

impl From<AnomalyGuardDto> for cu_core::AnomalyGuard {
    fn from(dto: AnomalyGuardDto) -> Self {
        let thresholds = cu_core::AnomalyThresholds {
            mass_modify_abs: dto.mass_modify_abs,
            mass_modify_multiplier: dto.mass_modify_multiplier,
            mass_delete_abs: dto.mass_delete_abs,
            mass_delete_multiplier: dto.mass_delete_multiplier,
            rename_min_count: dto.rename_min_count,
            rename_uniform_fraction_pct: dto.rename_uniform_fraction_pct,
            min_baseline_runs: dto.min_baseline_runs,
        };
        if dto.enabled {
            Self::Enabled(thresholds)
        } else {
            Self::Disabled
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecDto {
    program: String,
    args: Vec<String>,
    timeout_secs: u64,
}

impl From<cu_core::HookSpec> for HookSpecDto {
    fn from(hook: cu_core::HookSpec) -> Self {
        Self {
            program: hook.program.display().to_string(),
            args: hook.args,
            timeout_secs: hook.timeout_secs,
        }
    }
}

impl From<HookSpecDto> for cu_core::HookSpec {
    fn from(hook: HookSpecDto) -> Self {
        Self {
            program: hook.program.into(),
            args: hook.args,
            timeout_secs: hook.timeout_secs,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleWindowDto {
    days: u8,
    from_min: u16,
    to_min: u16,
}

impl From<cu_core::ScheduleWindow> for ScheduleWindowDto {
    fn from(w: cu_core::ScheduleWindow) -> Self {
        Self {
            days: w.days,
            from_min: w.from_min,
            to_min: w.to_min,
        }
    }
}

impl From<ScheduleWindowDto> for cu_core::ScheduleWindow {
    fn from(w: ScheduleWindowDto) -> Self {
        Self {
            days: w.days,
            from_min: w.from_min,
            to_min: w.to_min,
        }
    }
}

fn to_dto(spec: &cu_core::JobSpec, open_location: Option<&VaultLocation>) -> JobDto {
    JobDto {
        name: spec.name.clone(),
        enabled: spec.enabled,
        local_root: spec.local_root.display().to_string(),
        vault_path: spec.vault_path.clone(),
        interval_secs: spec.interval_secs,
        keep_versions: spec.keep_versions,
        keep_runs: spec.keep_runs,
        auto_unlock: spec.auto_unlock == cu_core::AutoUnlock::Enabled,
        matches_open_vault: open_location.is_some_and(|loc| vault_ref_matches(&spec.vault, loc)),
        excludes: spec.excludes.clone(),
        schedule_windows: spec
            .schedule_windows
            .iter()
            .copied()
            .map(ScheduleWindowDto::from)
            .collect(),
        pre_hook: spec.pre_hook.clone().map(HookSpecDto::from),
        post_hook: spec.post_hook.clone().map(HookSpecDto::from),
        anomaly_guard: AnomalyGuardDto::from(spec.anomaly_guard),
        timing: TimingDto {
            shuffle_uploads: spec.timing.shuffle_uploads,
            random_delay_max_secs: spec.timing.random_delay_max_secs,
        },
    }
}

#[tauri::command]
pub async fn job_list(state: State<'_, AppState>) -> Result<Vec<JobDto>, UiError> {
    let open_location = state
        .vault
        .lock()
        .await
        .as_ref()
        .map(|ov| ov.location.clone());
    let jobs = store(&state).load().map_err(|e| UiError::from_msg(&e))?;
    Ok(jobs
        .iter()
        .map(|spec| to_dto(spec, open_location.as_ref()))
        .collect())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn job_add(
    state: State<'_, AppState>,
    name: String,
    local_root: PathBuf,
    vault_path: String,
    interval_secs: u64,
    keep_versions: usize,
    keep_runs: u32,
) -> Result<(), UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    let local_root = std::fs::canonicalize(&local_root)
        .map_err(|e| UiError::coded(&Msg::new("JOB_LOCAL_DIR_INVALID").arg(&e)))?;
    let vault_ref = match &open_vault.location {
        VaultLocation::Local(root) => {
            let root = std::fs::canonicalize(root)
                .map_err(|e| UiError::coded(&Msg::new("JOB_VAULT_PATH_INVALID").arg(&e)))?;
            cu_core::VaultRef::Local(root)
        }
        VaultLocation::Remote(name) => cu_core::VaultRef::Remote(name.clone()),
    };
    store(&state)
        .add(cu_core::JobSpec {
            name,
            enabled: true,
            vault: vault_ref,
            local_root,
            vault_path,
            interval_secs,
            keep_versions,
            keep_runs,
            auto_unlock: cu_core::AutoUnlock::Disabled,
            excludes: Vec::new(),
            bandwidth_limit_kib: None,
            timing: cu_core::TimingPrivacy::default(),
            schedule_windows: Vec::new(),
            pre_hook: None,
            post_hook: None,
            anomaly_guard: cu_core::AnomalyGuard::default(),
        })
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_remove(state: State<'_, AppState>, name: String) -> Result<bool, UiError> {
    store(&state)
        .remove(&name)
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_set_enabled(
    state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<bool, UiError> {
    store(&state)
        .set_enabled(&name, enabled)
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_top_level_folders(
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<String>, UiError> {
    let jobs = store(&state).load().map_err(|e| UiError::from_msg(&e))?;
    let spec = jobs
        .into_iter()
        .find(|j| j.name == name)
        .ok_or_else(|| UiError::coded(&Msg::new("JOB_NOT_FOUND").arg(&name)))?;
    let entries = std::fs::read_dir(&spec.local_root).map_err(|e| {
        UiError::coded(
            &Msg::new("JOB_READ_DIR")
                .arg(spec.local_root.display())
                .arg(&e),
        )
    })?;
    let mut folders = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| UiError::from_msg(&e))?;
        let is_dir = entry
            .file_type()
            .map_err(|e| UiError::from_msg(&e))?
            .is_dir();
        if is_dir && let Some(name) = entry.file_name().to_str() {
            folders.push(name.to_string());
        }
    }
    folders.sort();
    Ok(folders)
}

#[tauri::command]
pub async fn job_set_excludes(
    state: State<'_, AppState>,
    name: String,
    excludes: Vec<String>,
) -> Result<bool, UiError> {
    store(&state)
        .set_excludes(&name, excludes)
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_set_schedule_windows(
    state: State<'_, AppState>,
    name: String,
    schedule_windows: Vec<ScheduleWindowDto>,
) -> Result<bool, UiError> {
    store(&state)
        .set_schedule_windows(
            &name,
            schedule_windows
                .into_iter()
                .map(cu_core::ScheduleWindow::from)
                .collect(),
        )
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_set_pre_hook(
    state: State<'_, AppState>,
    name: String,
    pre_hook: Option<HookSpecDto>,
) -> Result<bool, UiError> {
    store(&state)
        .set_pre_hook(&name, pre_hook.map(cu_core::HookSpec::from))
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_set_post_hook(
    state: State<'_, AppState>,
    name: String,
    post_hook: Option<HookSpecDto>,
) -> Result<bool, UiError> {
    store(&state)
        .set_post_hook(&name, post_hook.map(cu_core::HookSpec::from))
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_set_anomaly_guard(
    state: State<'_, AppState>,
    name: String,
    anomaly_guard: AnomalyGuardDto,
) -> Result<bool, UiError> {
    store(&state)
        .set_anomaly_guard(&name, cu_core::AnomalyGuard::from(anomaly_guard))
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_set_timing(
    state: State<'_, AppState>,
    name: String,
    timing: TimingDto,
) -> Result<bool, UiError> {
    if timing.random_delay_max_secs > cu_core::MAX_RANDOM_DELAY_SECS {
        return Err(UiError::coded(&Msg::new("JOB_JITTER_MAX")));
    }
    store(&state)
        .set_timing(
            &name,
            cu_core::TimingPrivacy {
                shuffle_uploads: timing.shuffle_uploads,
                random_delay_max_secs: timing.random_delay_max_secs,
            },
        )
        .map_err(|e| UiError::from_msg(&e))
}

#[tauri::command]
pub async fn job_enable_auto_unlock(
    state: State<'_, AppState>,
    name: String,
    password: String,
) -> Result<(), UiError> {
    let mut password = password;
    let job_store = store(&state);
    let forbidden = {
        let guard = state.vault.lock().await;
        guard.as_ref().is_some_and(|open| {
            open.vault.effective_policy().no_unlock_shortcuts
                && job_store.load().is_ok_and(|jobs| {
                    jobs.iter()
                        .any(|j| j.name == name && vault_ref_matches(&j.vault, &open.location))
                })
        })
    };
    if forbidden {
        password.zeroize();
        return Err(UiError::coded(&Msg::new("POLICY_NO_UNLOCK_SHORTCUTS")));
    }
    let result = cu_core::unlock::store_vault_password_in_keychain(&name, &password);
    password.zeroize();
    result.map_err(|e| UiError::from_msg(&e))?;

    let updated = job_store
        .set_auto_unlock(&name, cu_core::AutoUnlock::Enabled)
        .map_err(|e| UiError::from_msg(&e))?;
    if !updated {
        return Err(UiError::coded(&Msg::new("JOB_NOT_FOUND").arg(&name)));
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRowDto {
    run_id: i64,
    started_at: i64,
    finished_at: Option<i64>,
    outcome: Option<&'static str>,
    files_uploaded: u64,
    files_deleted: u64,
    files_renamed: u64,
    bytes_transferred: u64,
}

impl From<cu_core::RunRow> for RunRowDto {
    fn from(row: cu_core::RunRow) -> Self {
        Self {
            run_id: row.run_id,
            started_at: row.started_at,
            finished_at: row.finished_at,
            outcome: row.outcome.map(|o| match o {
                cu_core::RunOutcome::Ok => "ok",
                cu_core::RunOutcome::Failed => "failed",
                cu_core::RunOutcome::Interrupted => "interrupted",
            }),
            files_uploaded: row.files_uploaded,
            files_deleted: row.files_deleted,
            files_renamed: row.files_renamed,
            bytes_transferred: row.bytes_transferred,
        }
    }
}

#[tauri::command]
pub async fn job_history(
    state: State<'_, AppState>,
    name: String,
    limit: u32,
) -> Result<Vec<RunRowDto>, UiError> {
    let jobs = store(&state).load().map_err(|e| UiError::from_msg(&e))?;
    let spec = jobs
        .into_iter()
        .find(|j| j.name == name)
        .ok_or_else(|| UiError::coded(&Msg::new("JOB_NOT_FOUND").arg(&name)))?;
    Ok(cu_core::job_history(&state.data_dir, &spec, limit)?
        .into_iter()
        .map(Into::into)
        .collect())
}

#[derive(Clone, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum JobProgressEvent {
    Item {
        vault_path: String,
        op: &'static str,
        index: usize,
        total: usize,
    },
    Bytes {
        count: u64,
    },
    Plan {
        items: usize,
        upload_bytes: u64,
    },
}

impl From<cu_core::SyncProgress> for JobProgressEvent {
    fn from(event: cu_core::SyncProgress) -> Self {
        match event {
            cu_core::SyncProgress::Item {
                vault_path,
                op,
                index,
                total,
            } => Self::Item {
                vault_path,
                op: match op {
                    cu_core::SyncOp::SetAside => "set_aside",
                    cu_core::SyncOp::Mkdir => "mkdir",
                    cu_core::SyncOp::Upload => "upload",
                    cu_core::SyncOp::Rename => "rename",
                    cu_core::SyncOp::Delete => "delete",
                },
                index,
                total,
            },
            cu_core::SyncProgress::Bytes { count } => Self::Bytes { count },
            cu_core::SyncProgress::Plan {
                items,
                upload_bytes,
            } => Self::Plan {
                items,
                upload_bytes,
            },
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictInfo {
    vault_path: String,
    detail: String,
}

impl From<&cu_core::Conflict> for ConflictInfo {
    fn from(c: &cu_core::Conflict) -> Self {
        let detail = match &c.resolution {
            cu_core::ConflictResolution::KeptBoth { sidecar_path } => format!(
                "{}{}{}",
                c.reason,
                cu_msg::LIST_SEP,
                Msg::new("CONFLICT_KEPT_BOTH").arg(sidecar_path).wire()
            ),
            cu_core::ConflictResolution::Resurrected => format!(
                "{}{}{}",
                c.reason,
                cu_msg::LIST_SEP,
                Msg::new("CONFLICT_RESURRECTED").wire()
            ),
        };
        Self {
            vault_path: c.vault_path.clone(),
            detail,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PostHookOutcomeDto {
    Ok,
    Failed {
        exit_code: Option<i32>,
        stderr_tail: String,
    },
    TimedOut,
    SpawnFailed {
        reason: String,
    },
}

impl From<cu_core::HookOutcome> for PostHookOutcomeDto {
    fn from(outcome: cu_core::HookOutcome) -> Self {
        match outcome {
            cu_core::HookOutcome::Ok => Self::Ok,
            cu_core::HookOutcome::Failed {
                exit_code,
                stderr_tail,
            } => Self::Failed {
                exit_code,
                stderr_tail,
            },
            cu_core::HookOutcome::TimedOut => Self::TimedOut,
            cu_core::HookOutcome::SpawnFailed(reason) => Self::SpawnFailed { reason },
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummaryDto {
    uploads: u64,
    deletes: u64,
    renames: u64,
    bytes_transferred: u64,
    conflicts: Vec<ConflictInfo>,
    ignored_kept: usize,
    post_hook: Option<PostHookOutcomeDto>,
}

#[derive(serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum JobRunNowOutcome {
    Completed(RunSummaryDto),
    AnomalyBlocked {
        reason: String,
        uploads_affected: usize,
        deletes_affected: usize,
        renames_affected: usize,
    },
    Cancelled,
}

struct RunningJobGuard<'a> {
    state: &'a AppState,
    name: String,
}

impl Drop for RunningJobGuard<'_> {
    fn drop(&mut self) {
        self.state
            .running_jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.name);
    }
}

#[tauri::command]
pub async fn job_run_now(
    state: State<'_, AppState>,
    name: String,
    on_progress: Channel<JobProgressEvent>,
    confirm_anomaly: bool,
) -> Result<JobRunNowOutcome, UiError> {
    let (open_vault, generation) = state.take_vault().await.ok_or_else(UiError::no_vault)?;

    macro_rules! restore_and_fail {
        ($err:expr) => {{
            state.restore_vault(open_vault, generation).await;
            return Err($err);
        }};
    }

    let jobs = match store(&state).load() {
        Ok(jobs) => jobs,
        Err(e) => restore_and_fail!(UiError::from_msg(&e)),
    };
    let Some(spec) = jobs.into_iter().find(|j| j.name == name) else {
        restore_and_fail!(UiError::coded(&Msg::new("JOB_NOT_FOUND").arg(&name)));
    };
    if !vault_ref_matches(&spec.vault, &open_vault.location) {
        restore_and_fail!(UiError::coded(&Msg::new("JOB_OTHER_VAULT")));
    }

    let state_db = match cu_core::job_state_db_path(&state.data_dir, &spec) {
        Ok(path) => path,
        Err(e) => restore_and_fail!(UiError::from(e)),
    };
    if let Some(parent) = state_db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let job = cu_core::SyncJob::open(
        open_vault.vault,
        &state_db,
        &spec.local_root,
        &spec.vault_path,
        &state.data_dir,
    )?
    .with_retention(cu_core::RetentionPolicy {
        keep_versions: spec.keep_versions,
    })
    .with_extra_ignore(&spec.excludes)
    .with_job_name(spec.name.clone())
    .with_hooks(spec.pre_hook.as_ref(), spec.post_hook.as_ref())
    .with_anomaly_guard(&spec.anomaly_guard)
    .with_anomaly_override(confirm_anomaly);

    let cancel = cu_core::CancelToken::new();
    state
        .running_jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(spec.name.clone(), cancel.clone());
    let _running = RunningJobGuard {
        state: &state,
        name: spec.name.clone(),
    };
    let mut job = job.with_cancel_token(cancel);

    let mut on_event = |event: cu_core::SyncProgress| {
        let _ = on_progress.send(event.into());
    };
    let result = job.run_with_progress(&mut on_event).await;
    state
        .activity_log_for(&spec.vault)
        .record_run(&spec.name, &result);
    let _ = job.prune_run_history(spec.keep_runs);
    let vault = job.into_vault();
    state
        .restore_vault(
            OpenVault {
                vault,
                location: open_vault.location,
            },
            generation,
        )
        .await;

    match result {
        Ok(report) => Ok(JobRunNowOutcome::Completed(RunSummaryDto {
            uploads: report.stats.uploads,
            deletes: report.stats.deletes,
            renames: report.stats.renames,
            bytes_transferred: report.stats.bytes_transferred,
            conflicts: report.conflicts.iter().map(ConflictInfo::from).collect(),
            ignored_kept: report.ignored_kept.len(),
            post_hook: report.post_hook_outcome.map(PostHookOutcomeDto::from),
        })),
        Err(cu_core::CoreError::AnomalyDetected {
            reason,
            uploads_affected,
            deletes_affected,
            renames_affected,
        }) => Ok(JobRunNowOutcome::AnomalyBlocked {
            reason: cu_msg::wire_list(&reason),
            uploads_affected,
            deletes_affected,
            renames_affected,
        }),
        Err(cu_core::CoreError::Cancelled) => Ok(JobRunNowOutcome::Cancelled),
        Err(e) => Err(e.into()),
    }
}
