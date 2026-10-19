// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cu_format::config::VaultConfig;
use cu_index::{Db, RootId, RunOutcome};
use cu_vfs::VaultFs;

use crate::cuignore::CuIgnore;
use crate::diff::build_plan;
use crate::error::SyncError;
use crate::execute::{ExecutionStats, RetentionPolicy};
use crate::model::Plan;
use crate::progress::SyncProgress;

const CUIGNORE_FILE_NAME: &str = ".cuignore";

const ANOMALY_BASELINE_RUN_LIMIT: u32 = 50;

async fn load_cuignore(
    local_root: &Path,
    extra_patterns: &[String],
) -> Result<CuIgnore, SyncError> {
    let base = match tokio::fs::read_to_string(local_root.join(CUIGNORE_FILE_NAME)).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    if extra_patterns.is_empty() {
        return Ok(CuIgnore::parse(&base));
    }
    let mut combined = base;
    if !combined.is_empty() && !combined.ends_with('\n') {
        combined.push('\n');
    }
    combined.push_str(&extra_patterns.join("\n"));
    Ok(CuIgnore::parse(&combined))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
}

pub struct SyncEngine {
    db: Db,
    root_id: RootId,
    local_root: PathBuf,
    vault_path: String,
    retention: RetentionPolicy,
    device_label: Option<String>,
    head_db: Option<Db>,
    extra_ignore: Vec<String>,
    job_name: String,
    pre_hook: Option<crate::hooks::HookSpec>,
    post_hook: Option<crate::hooks::HookSpec>,
    anomaly_guard: crate::anomaly::AnomalyGuard,
    anomaly_override: bool,
    cancel: crate::progress::CancelToken,
    upload_concurrency: usize,
    shuffle_uploads: bool,
}

#[derive(Debug, Clone)]
pub struct SyncReport {
    pub run_id: i64,
    pub resumed_from_interruption: bool,
    pub stats: ExecutionStats,
    pub conflicts: Vec<crate::model::Conflict>,
    pub skipped: Vec<String>,
    pub ignored_kept: Vec<String>,
    pub retention_blocked: Vec<String>,
    pub post_hook_outcome: Option<crate::hooks::HookOutcome>,
}

impl SyncEngine {
    pub fn open(
        state_db_path: &Path,
        local_root: &Path,
        vault_path: &str,
    ) -> Result<Self, SyncError> {
        Self::over_db(Db::open(state_db_path)?, local_root, vault_path)
    }

    pub fn open_with_cipher(
        state_db_path: &Path,
        local_root: &Path,
        vault_path: &str,
        cipher: std::sync::Arc<dyn cu_index::FieldCipher>,
        sealed: bool,
    ) -> Result<Self, SyncError> {
        Self::over_db(
            Db::open_with_cipher(state_db_path, cipher, sealed)?,
            local_root,
            vault_path,
        )
    }

    fn over_db(db: Db, local_root: &Path, vault_path: &str) -> Result<Self, SyncError> {
        let root_id = db.get_or_create_root(&local_root.to_string_lossy(), vault_path, now_ms())?;
        Ok(Self {
            db,
            root_id,
            local_root: local_root.to_path_buf(),
            vault_path: vault_path.to_string(),
            retention: RetentionPolicy::default(),
            device_label: None,
            head_db: None,
            extra_ignore: Vec::new(),
            job_name: String::new(),
            pre_hook: None,
            post_hook: None,
            anomaly_guard: crate::anomaly::AnomalyGuard::default(),
            anomaly_override: false,
            cancel: crate::progress::CancelToken::new(),
            upload_concurrency: crate::execute::DEFAULT_UPLOAD_CONCURRENCY,
            shuffle_uploads: false,
        })
    }

    #[must_use]
    pub fn with_retention(mut self, retention: RetentionPolicy) -> Self {
        self.retention = retention;
        self
    }

    #[must_use]
    pub fn with_device_label(mut self, label: Option<String>) -> Self {
        self.device_label = label;
        self
    }

    #[must_use]
    pub fn with_head_db(mut self, head_db: Option<Db>) -> Self {
        self.head_db = head_db;
        self
    }

    #[must_use]
    pub fn with_extra_ignore(mut self, patterns: &[String]) -> Self {
        self.extra_ignore = patterns.to_vec();
        self
    }

    #[must_use]
    pub fn with_job_name(mut self, name: String) -> Self {
        self.job_name = name;
        self
    }

    #[must_use]
    pub fn with_hooks(
        mut self,
        pre: Option<crate::hooks::HookSpec>,
        post: Option<crate::hooks::HookSpec>,
    ) -> Self {
        self.pre_hook = pre;
        self.post_hook = post;
        self
    }

    #[must_use]
    pub fn with_anomaly_guard(mut self, guard: crate::anomaly::AnomalyGuard) -> Self {
        self.anomaly_guard = guard;
        self
    }

    #[must_use]
    pub fn with_anomaly_override(mut self, ignore_anomalies: bool) -> Self {
        self.anomaly_override = ignore_anomalies;
        self
    }

    #[must_use]
    pub fn with_shuffled_uploads(mut self, shuffle: bool) -> Self {
        self.shuffle_uploads = shuffle;
        self
    }

    #[must_use]
    pub fn with_cancel_token(mut self, cancel: crate::progress::CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    #[must_use]
    pub const fn root_id(&self) -> RootId {
        self.root_id
    }

    #[must_use]
    pub const fn db(&self) -> &Db {
        &self.db
    }

    #[must_use]
    pub fn vault_path(&self) -> &str {
        &self.vault_path
    }

    pub async fn dry_run(
        &mut self,
        vault_fs: &VaultFs,
        config: &VaultConfig,
    ) -> Result<Plan, SyncError> {
        let ignore = load_cuignore(&self.local_root, &self.extra_ignore).await?;
        let all_nodes = self.db.list_all_nodes(self.root_id)?;
        build_plan(
            &self.local_root,
            &self.vault_path,
            vault_fs,
            config,
            all_nodes,
            now_ns(),
            &ignore,
            false,
            self.device_label.as_deref(),
            now_ms(),
        )
        .await
    }

    pub async fn run(
        &mut self,
        vault_fs: &VaultFs,
        config: &VaultConfig,
    ) -> Result<SyncReport, SyncError> {
        self.run_inner(vault_fs, config, false, &mut |_| {}).await
    }

    pub async fn run_with_progress(
        &mut self,
        vault_fs: &VaultFs,
        config: &VaultConfig,
        on_progress: &mut (dyn FnMut(SyncProgress) + Send),
    ) -> Result<SyncReport, SyncError> {
        self.run_inner(vault_fs, config, false, on_progress).await
    }

    pub async fn forget_ignored(
        &mut self,
        vault_fs: &VaultFs,
        config: &VaultConfig,
    ) -> Result<SyncReport, SyncError> {
        self.run_inner(vault_fs, config, true, &mut |_| {}).await
    }

    fn hook_env(
        &self,
        phase: &'static str,
        outcome: Option<(&ExecutionStats, usize, usize)>,
    ) -> Vec<(&'static str, String)> {
        let mut vars = vec![
            ("CU_JOB_NAME", self.job_name.clone()),
            ("CU_LOCAL_ROOT", self.local_root.display().to_string()),
            ("CU_VAULT_PATH", self.vault_path.clone()),
            ("CU_RUN_PHASE", phase.to_string()),
        ];
        if let Some((stats, conflicts, ignored_kept)) = outcome {
            vars.push(("CU_RUN_OUTCOME", "ok".to_string()));
            vars.push(("CU_UPLOADED", stats.uploads.to_string()));
            vars.push(("CU_DELETED", stats.deletes.to_string()));
            vars.push(("CU_RENAMED", stats.renames.to_string()));
            vars.push(("CU_BYTES_TRANSFERRED", stats.bytes_transferred.to_string()));
            vars.push(("CU_CONFLICTS", conflicts.to_string()));
            vars.push(("CU_IGNORED_KEPT", ignored_kept.to_string()));
        }
        vars
    }

    #[allow(clippy::too_many_lines)]
    async fn run_inner(
        &mut self,
        vault_fs: &VaultFs,
        config: &VaultConfig,
        forget_ignored: bool,
        on_progress: &mut (dyn FnMut(SyncProgress) + Send),
    ) -> Result<SyncReport, SyncError> {
        crate::manifest_log::check_run_start(
            vault_fs,
            config,
            &mut crate::manifest::HeadStores {
                job: Some(&mut self.db),
                shared: self.head_db.as_mut(),
            },
        )
        .await?;

        let run_started_at_ms = now_ms();
        let run_id = self.db.start_run(self.root_id, run_started_at_ms)?;

        let resumed_from_interruption = match self.mark_previous_runs_interrupted(run_id) {
            Ok(resumed) => resumed,
            Err(e) => return self.fail_run(run_id, e),
        };

        if let Some(pre) = &self.pre_hook {
            let outcome = crate::hooks::run_hook(pre, &self.hook_env("pre", None)).await;
            if outcome != crate::hooks::HookOutcome::Ok {
                return self.fail_run(run_id, SyncError::PreHookFailed(outcome));
            }
        }

        let (mut plan, known_extensions) = match self
            .prepare_plan(vault_fs, config, forget_ignored, run_started_at_ms)
            .await
        {
            Ok(prepared) => prepared,
            Err(e) => return self.fail_run(run_id, e),
        };

        if !self.anomaly_override
            && let crate::anomaly::AnomalyGuard::Enabled(thresholds) = &self.anomaly_guard
        {
            let runs = match self.db.list_runs(self.root_id, ANOMALY_BASELINE_RUN_LIMIT) {
                Ok(runs) => runs,
                Err(e) => return self.fail_run(run_id, e.into()),
            };
            let baseline: Vec<crate::anomaly::RunStats> = runs
                .iter()
                .filter(|run| run.outcome == Some(RunOutcome::Ok))
                .map(crate::anomaly::RunStats::from)
                .collect();
            if let Some(report) =
                crate::anomaly::detect(&plan, &baseline, &known_extensions, thresholds)
            {
                return self.fail_run(
                    run_id,
                    SyncError::AnomalyDetected {
                        reason: report.reason,
                        uploads_affected: report.uploads_affected,
                        deletes_affected: report.deletes_affected,
                        renames_affected: report.renames_affected,
                    },
                );
            }
        }

        let exec_result = crate::execute::execute_plan_tuned(
            &mut self.db,
            vault_fs,
            config,
            self.root_id,
            run_id,
            &plan,
            self.retention,
            now_ms,
            on_progress,
            self.device_label.as_deref(),
            &self.cancel,
            crate::execute::ExecuteOptions {
                upload_concurrency: self.upload_concurrency,
                shuffle_uploads: self.shuffle_uploads,
            },
        )
        .await;
        let crate::execute::ExecutionOutcome {
            stats,
            conflicts: extra_conflicts,
            retention_blocked,
            skipped: exec_skipped,
        } = match exec_result {
            Ok(result) => result,
            Err(e) => return self.fail_run(run_id, e),
        };
        plan.conflicts.extend(extra_conflicts);
        plan.skipped.extend(exec_skipped);

        self.db.finish_run(
            run_id,
            now_ms(),
            RunOutcome::Ok,
            stats.uploads,
            stats.deletes,
            stats.renames,
            stats.bytes_transferred,
        )?;
        self.db.set_run_modified(run_id, stats.modifies)?;

        let post_hook_outcome = match &self.post_hook {
            Some(post) => {
                let env = self.hook_env(
                    "post",
                    Some((&stats, plan.conflicts.len(), plan.ignored_kept.len())),
                );
                Some(crate::hooks::run_hook(post, &env).await)
            }
            None => None,
        };

        Ok(SyncReport {
            run_id,
            resumed_from_interruption,
            stats,
            conflicts: plan.conflicts,
            skipped: plan.skipped,
            ignored_kept: plan.ignored_kept,
            retention_blocked,
            post_hook_outcome,
        })
    }

    pub async fn prune_versions_now(
        &mut self,
        vault_fs: &VaultFs,
        config: &VaultConfig,
        retention: RetentionPolicy,
    ) -> Result<(u64, Vec<String>), SyncError> {
        let excess = self
            .db
            .versions_beyond_limit_for_root(self.root_id, retention.keep_versions)?;
        let mut retention_blocked = Vec::new();
        let pruned = crate::execute::forget_archived_versions(
            &mut self.db,
            vault_fs,
            config,
            &excess,
            &mut retention_blocked,
        )
        .await?;
        Ok((pruned, retention_blocked))
    }

    pub fn count_versions_beyond(&self, retention: RetentionPolicy) -> Result<u64, SyncError> {
        Ok(self
            .db
            .count_versions_beyond_for_root(self.root_id, retention.keep_versions)?)
    }

    pub fn count_runs_beyond(&self, keep_runs: u32) -> Result<u64, SyncError> {
        Ok(u64::try_from(
            self.db
                .ok_runs_beyond_limit(self.root_id, keep_runs as usize)?
                .len(),
        )
        .unwrap_or(0))
    }

    pub fn prune_run_history(&self, keep_runs: u32) -> Result<u64, SyncError> {
        let run_ids = self
            .db
            .ok_runs_beyond_limit(self.root_id, keep_runs as usize)?;
        self.db.in_transaction(|| {
            for &run_id in &run_ids {
                self.db.delete_run(run_id)?;
            }
            Ok(())
        })?;
        Ok(u64::try_from(run_ids.len()).unwrap_or(u64::MAX))
    }

    fn fail_run<T>(&self, run_id: i64, err: SyncError) -> Result<T, SyncError> {
        let outcome = if matches!(err, SyncError::Cancelled) {
            RunOutcome::Interrupted
        } else {
            RunOutcome::Failed
        };
        self.db.finish_run(run_id, now_ms(), outcome, 0, 0, 0, 0)?;
        Err(err)
    }

    fn ensure_names_nfc(&self) -> Result<Vec<String>, SyncError> {
        if self.db.names_normalized(self.root_id)? {
            return Ok(Vec::new());
        }
        let report = self.db.normalize_names(self.root_id, &|name| {
            let nfc = cu_format::name::nfc(name);
            (nfc != name).then(|| nfc.into_owned())
        })?;
        let mut notes = Vec::new();
        if report.forgotten > 0 {
            notes.push(
                cu_msg::Msg::new("SYNC_NFC_FORGOTTEN")
                    .arg(report.forgotten)
                    .wire(),
            );
        }
        Ok(notes)
    }

    async fn prepare_plan(
        &mut self,
        vault_fs: &VaultFs,
        config: &VaultConfig,
        forget_ignored: bool,
        run_started_at_ms: i64,
    ) -> Result<(Plan, std::collections::HashSet<String>), SyncError> {
        let nfc_notes = self.ensure_names_nfc()?;
        if !self.vault_path.is_empty() {
            vault_fs.mkdir(config, &self.vault_path).await?;
        }
        let ignore = load_cuignore(&self.local_root, &self.extra_ignore).await?;
        let scan_started_at_ns = now_ns();
        let all_nodes = self.db.list_all_nodes(self.root_id)?;
        let known_extensions = crate::anomaly::known_extensions(&all_nodes);
        let mut plan = build_plan(
            &self.local_root,
            &self.vault_path,
            vault_fs,
            config,
            all_nodes,
            scan_started_at_ns,
            &ignore,
            forget_ignored,
            self.device_label.as_deref(),
            run_started_at_ms,
        )
        .await?;
        plan.skipped.extend(nfc_notes);
        Ok((plan, known_extensions))
    }

    fn mark_previous_runs_interrupted(&self, current_run_id: i64) -> Result<bool, SyncError> {
        Ok(self
            .db
            .mark_unfinished_runs_interrupted(self.root_id, current_run_id, now_ms())?
            > 0)
    }
}
