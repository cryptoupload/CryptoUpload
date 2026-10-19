// SPDX-License-Identifier: GPL-3.0-or-later

pub mod error;
pub mod job;
pub mod manager;
pub mod scheduler;
pub mod status;
pub mod store;
pub mod watcher;

pub use error::JobsError;
pub use job::MAX_RANDOM_DELAY_SECS;
pub use job::{
    AnomalyGuard, AnomalyThresholds, AutoUnlock, HookSpec, JobError, JobRunner, JobSpec,
    PostHookOutcome, RunSummary, ScheduleWindow, TimingPrivacy, VaultRef, window_allows,
};
pub use manager::JobManager;
pub use status::{JobStatus, Outcome, StatusStore};
pub use store::{JobStore, invalid_job_name};
pub use watcher::WatchHealth;
