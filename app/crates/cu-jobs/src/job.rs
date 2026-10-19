// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VaultRef {
    Local(PathBuf),
    Remote(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AutoUnlock {
    #[default]
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSpec {
    pub name: String,
    pub enabled: bool,
    pub vault: VaultRef,
    pub local_root: PathBuf,
    pub vault_path: String,
    pub interval_secs: u64,
    pub keep_versions: usize,
    pub keep_runs: u32,
    pub auto_unlock: AutoUnlock,
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub bandwidth_limit_kib: Option<u32>,
    #[serde(default)]
    pub schedule_windows: Vec<ScheduleWindow>,
    #[serde(default)]
    pub pre_hook: Option<HookSpec>,
    #[serde(default)]
    pub post_hook: Option<HookSpec>,
    #[serde(default)]
    pub anomaly_guard: AnomalyGuard,
    #[serde(default)]
    pub timing: TimingPrivacy,
}

pub const MAX_RANDOM_DELAY_SECS: u32 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingPrivacy {
    #[serde(default)]
    pub shuffle_uploads: bool,
    #[serde(default)]
    pub random_delay_max_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSpec {
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleWindow {
    pub days: u8,
    pub from_min: u16,
    pub to_min: u16,
}

#[must_use]
pub fn window_allows(windows: &[ScheduleWindow], weekday: u8, minute_of_day: u16) -> bool {
    if windows.is_empty() {
        return true;
    }
    let previous_weekday = (weekday + 6) % 7;
    windows.iter().any(|w| {
        let today_bit = (w.days >> weekday) & 1 != 0;
        let yesterday_bit = (w.days >> previous_weekday) & 1 != 0;
        if w.from_min < w.to_min {
            today_bit && minute_of_day >= w.from_min && minute_of_day < w.to_min
        } else {
            (today_bit && minute_of_day >= w.from_min)
                || (yesterday_bit && minute_of_day < w.to_min)
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnomalyGuard {
    Disabled,
    Enabled(AnomalyThresholds),
}

impl Default for AnomalyGuard {
    fn default() -> Self {
        Self::Enabled(AnomalyThresholds::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnomalyThresholds {
    pub mass_modify_abs: u64,
    pub mass_modify_multiplier: u64,
    pub mass_delete_abs: u64,
    pub mass_delete_multiplier: u64,
    pub rename_min_count: usize,
    pub rename_uniform_fraction_pct: u8,
    pub min_baseline_runs: usize,
}

impl AnomalyThresholds {
    pub fn validate(&self) -> Result<(), Msg> {
        let at_least_one = [
            ("JOBS_THRESHOLD_MODIFY_ABS", self.mass_modify_abs),
            ("JOBS_THRESHOLD_MODIFY_MULT", self.mass_modify_multiplier),
            ("JOBS_THRESHOLD_DELETE_ABS", self.mass_delete_abs),
            ("JOBS_THRESHOLD_DELETE_MULT", self.mass_delete_multiplier),
            ("JOBS_THRESHOLD_RENAME_MIN", self.rename_min_count as u64),
        ];
        for (code, value) in at_least_one {
            if value == 0 {
                return Err(Msg::new(code));
            }
        }
        if !(1..=100).contains(&self.rename_uniform_fraction_pct) {
            return Err(Msg::new("JOBS_THRESHOLD_PCT").arg(self.rename_uniform_fraction_pct));
        }
        Ok(())
    }
}

impl Default for AnomalyThresholds {
    fn default() -> Self {
        Self {
            mass_modify_abs: 200,
            mass_modify_multiplier: 5,
            mass_delete_abs: 100,
            mass_delete_multiplier: 5,
            rename_min_count: 20,
            rename_uniform_fraction_pct: 80,
            min_baseline_runs: 5,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunSummary {
    pub uploaded: u64,
    pub deleted: u64,
    pub renamed: u64,
    pub bytes_transferred: u64,
    pub conflicts: usize,
    pub ignored_kept: usize,
    pub post_hook: Option<PostHookOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostHookOutcome {
    Ok,
    Failed(String),
}

#[derive(Debug)]
pub enum JobError {
    PasswordUnavailable,
    Failed(Msg),
}

impl Coded for JobError {
    fn msg(&self) -> Msg {
        match self {
            Self::PasswordUnavailable => Msg::new("JOBS_PASSWORD_UNAVAILABLE"),
            Self::Failed(msg) => msg.clone(),
        }
    }
}

display_as_msg!(JobError);

impl std::error::Error for JobError {}

#[async_trait::async_trait]
pub trait JobRunner: Send + Sync {
    async fn run_once(&self, spec: &JobSpec) -> Result<RunSummary, JobError>;
}
