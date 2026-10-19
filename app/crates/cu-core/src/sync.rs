// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use cu_index::{Db, NodeId, RootId, RunRow, VersionRow};
use cu_sync::engine::{SyncEngine, SyncReport};
use cu_sync::verify::VerifyIssue;
use cu_sync::{restore, verify};

use crate::error::CoreError;
use crate::vault::Vault;

pub struct SyncJob {
    vault: Vault,
    engine: SyncEngine,
}

impl SyncJob {
    pub fn open(
        vault: Vault,
        state_db_path: &Path,
        local_root: &Path,
        vault_path: &str,
        data_dir: &Path,
    ) -> Result<Self, CoreError> {
        let identity = crate::device::current(data_dir)?;
        let head_db = crate::registry::open_head_db(&vault, data_dir)?;
        let access = vault.local_state()?;
        let engine = match access.cipher() {
            Some(cipher) => SyncEngine::open_with_cipher(
                state_db_path,
                local_root,
                vault_path,
                cipher,
                access.encrypt,
            )?,
            None => SyncEngine::open(state_db_path, local_root, vault_path)?,
        }
        .with_device_label(Some(identity.label))
        .with_head_db(Some(head_db));
        Ok(Self { vault, engine })
    }

    #[must_use]
    pub fn with_retention(mut self, retention: cu_sync::RetentionPolicy) -> Self {
        self.engine = self.engine.with_retention(retention);
        self
    }

    #[must_use]
    pub fn with_extra_ignore(mut self, patterns: &[String]) -> Self {
        self.engine = self.engine.with_extra_ignore(patterns);
        self
    }

    #[must_use]
    pub fn with_job_name(mut self, name: String) -> Self {
        self.engine = self.engine.with_job_name(name);
        self
    }

    #[must_use]
    pub fn with_hooks(
        mut self,
        pre: Option<&cu_jobs::HookSpec>,
        post: Option<&cu_jobs::HookSpec>,
    ) -> Self {
        self.engine = self
            .engine
            .with_hooks(pre.map(to_sync_hook), post.map(to_sync_hook));
        self
    }

    #[must_use]
    pub fn with_anomaly_guard(mut self, guard: &cu_jobs::AnomalyGuard) -> Self {
        self.engine = self.engine.with_anomaly_guard(to_sync_anomaly_guard(guard));
        self
    }

    #[must_use]
    pub fn with_shuffled_uploads(mut self, shuffle: bool) -> Self {
        self.engine = self.engine.with_shuffled_uploads(shuffle);
        self
    }

    #[must_use]
    pub fn with_anomaly_override(mut self, ignore_anomalies: bool) -> Self {
        self.engine = self.engine.with_anomaly_override(ignore_anomalies);
        self
    }

    #[must_use]
    pub fn with_cancel_token(mut self, cancel: cu_sync::CancelToken) -> Self {
        self.engine = self.engine.with_cancel_token(cancel);
        self
    }

    pub async fn dry_run(&mut self) -> Result<cu_sync::Plan, CoreError> {
        Ok(self
            .engine
            .dry_run(self.vault.fs(), self.vault.config())
            .await?)
    }

    pub async fn run(&mut self) -> Result<SyncReport, CoreError> {
        let result = self.engine.run(self.vault.fs(), self.vault.config()).await;
        self.vault.settled(result.map_err(Into::into)).await
    }

    pub async fn run_with_progress(
        &mut self,
        on_progress: &mut (dyn FnMut(cu_sync::SyncProgress) + Send),
    ) -> Result<SyncReport, CoreError> {
        let result = self
            .engine
            .run_with_progress(self.vault.fs(), self.vault.config(), on_progress)
            .await;
        self.vault.settled(result.map_err(Into::into)).await
    }

    pub async fn forget_ignored(&mut self) -> Result<SyncReport, CoreError> {
        let result = self
            .engine
            .forget_ignored(self.vault.fs(), self.vault.config())
            .await;
        self.vault.settled(result.map_err(Into::into)).await
    }

    pub fn prune_run_history(&self, keep_runs: u32) -> Result<u64, CoreError> {
        Ok(self.engine.prune_run_history(keep_runs)?)
    }

    pub fn count_versions_beyond(
        &self,
        retention: cu_sync::RetentionPolicy,
    ) -> Result<u64, CoreError> {
        Ok(self.engine.count_versions_beyond(retention)?)
    }

    pub fn count_runs_beyond(&self, keep_runs: u32) -> Result<u64, CoreError> {
        Ok(self.engine.count_runs_beyond(keep_runs)?)
    }

    pub async fn prune_versions_now(
        &mut self,
        retention: cu_sync::RetentionPolicy,
    ) -> Result<(u64, Vec<String>), CoreError> {
        let result = self
            .engine
            .prune_versions_now(self.vault.fs(), self.vault.config(), retention)
            .await;
        self.vault.settled(result.map_err(Into::into)).await
    }

    #[must_use]
    pub fn db(&self) -> &Db {
        self.engine.db()
    }

    #[must_use]
    pub const fn root_id(&self) -> RootId {
        self.engine.root_id()
    }

    #[must_use]
    pub fn into_vault(self) -> Vault {
        self.vault
    }

    pub fn local_state(&self) -> Result<crate::LocalStateAccess, CoreError> {
        self.vault.local_state()
    }

    pub fn history(&self, limit: u32) -> Result<Vec<RunRow>, CoreError> {
        Ok(self.db().list_runs(self.root_id(), limit)?)
    }

    pub fn list_versions(&self, vault_path: &str) -> Result<Vec<VersionRow>, CoreError> {
        let node_id = self.resolve_node_id(vault_path)?;
        Ok(self.db().list_versions(node_id)?)
    }

    pub async fn restore_version(
        &mut self,
        vault_path: &str,
        version: i64,
        local_dest: &Path,
    ) -> Result<(), CoreError> {
        let node_id = self.resolve_node_id(vault_path)?;
        let source_path =
            restore::resolve_restore_source(self.db(), self.engine.vault_path(), node_id, version)?;
        restore::write_restored_file(
            self.vault.fs(),
            self.vault.config(),
            &source_path,
            local_dest,
        )
        .await?;
        Ok(())
    }

    pub async fn restore_before(
        &mut self,
        vault_path: &str,
        at_ms: i64,
        local_dest: &Path,
    ) -> Result<(), CoreError> {
        let node_id = self.resolve_node_id(vault_path)?;
        let source_path = restore::resolve_restore_source_before(
            self.db(),
            self.engine.vault_path(),
            node_id,
            at_ms,
        )?;
        restore::write_restored_file(
            self.vault.fs(),
            self.vault.config(),
            &source_path,
            local_dest,
        )
        .await?;
        Ok(())
    }

    pub async fn verify_existence(&self) -> Result<Vec<VerifyIssue>, CoreError> {
        Ok(verify::verify_existence(
            self.db(),
            self.vault.fs(),
            self.vault.config(),
            self.engine.vault_path(),
            self.root_id(),
        )
        .await?)
    }

    pub async fn verify_content(&self) -> Result<Vec<VerifyIssue>, CoreError> {
        Ok(verify::verify_content(
            self.db(),
            self.vault.fs(),
            self.vault.config(),
            self.engine.vault_path(),
            self.root_id(),
        )
        .await?)
    }

    fn resolve_node_id(&self, vault_path: &str) -> Result<NodeId, CoreError> {
        let segments: Vec<&str> = vault_path
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        let Some((last, ancestors)) = segments.split_last() else {
            return Err(CoreError::NotFound);
        };

        let mut current: Option<NodeId> = None;
        for segment in ancestors {
            let node = self
                .db()
                .find_child(self.root_id(), current, segment)?
                .ok_or(CoreError::NotFound)?;
            current = Some(node.node_id);
        }

        if let Some(node) = self.db().find_child(self.root_id(), current, last)? {
            return Ok(node.node_id);
        }
        self.db()
            .find_node_id_in_history(current, last)?
            .ok_or(CoreError::NotFound)
    }
}

fn to_sync_hook(hook: &cu_jobs::HookSpec) -> cu_sync::hooks::HookSpec {
    cu_sync::hooks::HookSpec {
        program: hook.program.clone(),
        args: hook.args.clone(),
        timeout_secs: hook.timeout_secs,
    }
}

fn to_sync_anomaly_guard(guard: &cu_jobs::AnomalyGuard) -> cu_sync::anomaly::AnomalyGuard {
    match guard {
        cu_jobs::AnomalyGuard::Disabled => cu_sync::anomaly::AnomalyGuard::Disabled,
        cu_jobs::AnomalyGuard::Enabled(t) => {
            cu_sync::anomaly::AnomalyGuard::Enabled(cu_sync::anomaly::AnomalyThresholds {
                mass_modify_abs: t.mass_modify_abs,
                mass_modify_multiplier: t.mass_modify_multiplier,
                mass_delete_abs: t.mass_delete_abs,
                mass_delete_multiplier: t.mass_delete_multiplier,
                rename_min_count: t.rename_min_count,
                rename_uniform_fraction_pct: t.rename_uniform_fraction_pct,
                min_baseline_runs: t.min_baseline_runs,
            })
        }
    }
}
