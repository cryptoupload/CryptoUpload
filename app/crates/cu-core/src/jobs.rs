// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg};
use std::path::{Path, PathBuf};

use cu_jobs::{AutoUnlock, JobError, JobRunner, JobSpec, RunSummary, VaultRef};

use crate::error::CoreError;
use crate::remote::{self};
use crate::sync::SyncJob;
use crate::unlock;
use crate::vault::Vault;

pub struct SyncJobRunner {
    data_dir: PathBuf,
}

impl SyncJobRunner {
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    fn secrets_file_path(&self) -> PathBuf {
        crate::paths::secrets_file_path(&self.data_dir)
    }

    async fn open_vault(&self, spec: &JobSpec, password: &[u8]) -> Result<Vault, JobError> {
        let credential = crate::Credential::Password(password);
        match &spec.vault {
            VaultRef::Local(root) => {
                Vault::open_local_with_bandwidth(root, &credential, spec.bandwidth_limit_kib)
                    .await
                    .map_err(|e| describe(&e))
            }
            VaultRef::Remote(name) => {
                let secrets_password = unlock::try_unlock_remote_password(name)
                    .ok_or(JobError::PasswordUnavailable)?;
                match crate::SecretsFile::format_version(&self.secrets_file_path()) {
                    Ok(cu_secrets::FileFormat::LegacyV1) => {
                        return Err(JobError::Failed(Msg::new("SECRETS_LEGACY_FORMAT")));
                    }
                    Ok(_) => {}
                    Err(e) => return Err(JobError::Failed(e.msg())),
                }
                let secrets = crate::SecretsFile::load(&self.secrets_file_path())
                    .map_err(|e| JobError::Failed(e.msg()))?;
                let cfg = remote::unlock_remote(&secrets, name, secrets_password.as_bytes())
                    .map_err(|e| JobError::Failed(e.msg()))?;
                let remote_state_db =
                    crate::resume::remote_state_db_path(&self.data_dir, &remote::remote_id(&cfg));
                if let Some(parent) = remote_state_db.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let refresh_sink = remote::persisting_refresh_sink(
                    self.secrets_file_path(),
                    name.clone(),
                    secrets_password.as_bytes().to_vec(),
                );
                Vault::open_remote_with_refresh_sink(
                    &cfg,
                    &remote_state_db,
                    &credential,
                    spec.bandwidth_limit_kib,
                    Some(refresh_sink),
                )
                .await
                .map_err(|e| describe(&e))
            }
        }
    }
}

pub fn sync_state_db_path(
    data_dir: &Path,
    vault: &VaultRef,
    local_root: &Path,
    vault_path: &str,
) -> Result<PathBuf, CoreError> {
    let mut hasher = cu_crypto::hash::Hasher::new();
    match vault {
        VaultRef::Local(root) => {
            let canonical = std::fs::canonicalize(root)?;
            hasher.update(canonical.to_string_lossy().as_bytes());
        }
        VaultRef::Remote(name) => {
            hasher.update(name.as_bytes());
        }
    }
    hasher.update(&[0]);
    let canonical_local = std::fs::canonicalize(local_root)?;
    hasher.update(canonical_local.to_string_lossy().as_bytes());
    hasher.update(&[0]);
    hasher.update(vault_path.as_bytes());
    let name = format!("{}.db", hex_digest(&hasher.finalize()));
    Ok(data_dir.join("cryptoupload").join("sync").join(name))
}

fn hex_digest(bytes: &[u8; 32]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(64), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

pub fn job_state_db_path(data_dir: &Path, spec: &JobSpec) -> Result<PathBuf, CoreError> {
    sync_state_db_path(data_dir, &spec.vault, &spec.local_root, &spec.vault_path)
}

fn describe(e: &CoreError) -> JobError {
    JobError::Failed(e.msg())
}

#[async_trait::async_trait]
impl JobRunner for SyncJobRunner {
    async fn run_once(&self, spec: &JobSpec) -> Result<RunSummary, JobError> {
        if spec.auto_unlock != AutoUnlock::Enabled {
            return Err(JobError::PasswordUnavailable);
        }
        let password =
            unlock::try_unlock_vault_password(&spec.name).ok_or(JobError::PasswordUnavailable)?;

        let vault = self.open_vault(spec, password.as_bytes()).await?;
        if vault.effective_policy().no_unlock_shortcuts {
            return Err(JobError::Failed(
                CoreError::PolicyForbids(Msg::new("POLICY_NO_UNLOCK_SHORTCUTS")).msg(),
            ));
        }

        let local_state = vault.local_state().map_err(|e| describe(&e))?;
        let state_db = job_state_db_path(&self.data_dir, spec).map_err(|e| describe(&e))?;
        if let Some(parent) = state_db.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut job = SyncJob::open(
            vault,
            &state_db,
            &spec.local_root,
            &spec.vault_path,
            &self.data_dir,
        )
        .map_err(|e| describe(&e))?
        .with_retention(cu_sync::RetentionPolicy {
            keep_versions: spec.keep_versions,
        })
        .with_extra_ignore(&spec.excludes)
        .with_job_name(spec.name.clone())
        .with_hooks(spec.pre_hook.as_ref(), spec.post_hook.as_ref())
        .with_anomaly_guard(&spec.anomaly_guard)
        .with_shuffled_uploads(spec.timing.shuffle_uploads);

        let result = job.run().await;
        crate::activity::ActivityLog::for_vault(&self.data_dir, &spec.vault, &local_state)
            .record_run(&spec.name, &result);
        let report = result.map_err(|e| describe(&e))?;
        let _ = job.prune_run_history(spec.keep_runs);

        Ok(RunSummary {
            uploaded: report.stats.uploads,
            deleted: report.stats.deletes,
            renamed: report.stats.renames,
            bytes_transferred: report.stats.bytes_transferred,
            conflicts: report.conflicts.len(),
            ignored_kept: report.ignored_kept.len(),
            post_hook: report.post_hook_outcome.map(|outcome| match outcome {
                cu_sync::hooks::HookOutcome::Ok => cu_jobs::PostHookOutcome::Ok,
                other => cu_jobs::PostHookOutcome::Failed(other.post_hook_msg().wire()),
            }),
        })
    }
}

pub fn job_history(
    data_dir: &Path,
    spec: &JobSpec,
    limit: u32,
) -> Result<Vec<cu_index::RunRow>, CoreError> {
    let path = job_state_db_path(data_dir, spec)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let db = cu_index::Db::open(&path)?;
    let Some(root_id) = db.find_root(&spec.local_root.to_string_lossy(), &spec.vault_path)? else {
        return Ok(Vec::new());
    };
    Ok(db.list_runs(root_id, limit)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobUsage {
    pub job_name: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub version_count: u64,
    pub version_bytes: u64,
    pub total_bytes_transferred_lifetime: u64,
}

pub fn storage_usage(data_dir: &Path) -> Result<Vec<JobUsage>, CoreError> {
    let store = cu_jobs::JobStore::new(default_jobs_path(data_dir));
    let specs = store.load()?;
    let mut usages = Vec::with_capacity(specs.len());
    for spec in &specs {
        let path = job_state_db_path(data_dir, spec)?;
        let usage = if path.exists() {
            let db = cu_index::Db::open(&path)?;
            match db.find_root(&spec.local_root.to_string_lossy(), &spec.vault_path)? {
                Some(root_id) => {
                    let (file_count, total_bytes) = db.root_file_totals(root_id)?;
                    let (version_count, version_bytes) =
                        db.root_archived_version_totals(root_id)?;
                    let total_bytes_transferred_lifetime = db.sum_bytes_transferred_ok(root_id)?;
                    JobUsage {
                        job_name: spec.name.clone(),
                        file_count,
                        total_bytes,
                        version_count,
                        version_bytes,
                        total_bytes_transferred_lifetime,
                    }
                }
                None => zero_usage(spec.name.clone()),
            }
        } else {
            zero_usage(spec.name.clone())
        };
        usages.push(usage);
    }
    Ok(usages)
}

fn zero_usage(job_name: String) -> JobUsage {
    JobUsage {
        job_name,
        file_count: 0,
        total_bytes: 0,
        version_count: 0,
        version_bytes: 0,
        total_bytes_transferred_lifetime: 0,
    }
}

pub fn revoke_auto_unlock(
    jobs_path: &Path,
    is_this_vault: impl Fn(&VaultRef) -> bool,
) -> Result<Vec<String>, CoreError> {
    let store = cu_jobs::JobStore::new(jobs_path.to_path_buf());
    let jobs = store.load().map_err(|e| CoreError::Jobs(e.msg()))?;
    let mut revoked = Vec::new();
    for spec in jobs {
        if spec.auto_unlock != AutoUnlock::Enabled || !is_this_vault(&spec.vault) {
            continue;
        }
        store
            .set_auto_unlock(&spec.name, AutoUnlock::Disabled)
            .map_err(|e| CoreError::Jobs(e.msg()))?;
        let _ = unlock::forget_vault_password_in_keychain(&spec.name);
        revoked.push(spec.name);
    }
    Ok(revoked)
}

#[must_use]
pub fn default_status_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join("jobs").join("status")
}

#[must_use]
pub fn default_jobs_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cryptoupload").join("jobs.json")
}
