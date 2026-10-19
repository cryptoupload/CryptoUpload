// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{ErrorKind, Write as _};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::job::PostHookOutcome;
use crate::watcher::WatchHealth;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome")]
pub enum Outcome {
    Ok {
        uploaded: u64,
        deleted: u64,
        renamed: u64,
        bytes_transferred: u64,
        conflicts: usize,
        ignored_kept: usize,
        #[serde(default)]
        post_hook: Option<PostHookOutcome>,
    },
    PasswordUnavailable,
    OutsideWindow,
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    pub last_run_started_at_ms: Option<i64>,
    pub last_run_finished_at_ms: Option<i64>,
    pub last_outcome: Option<Outcome>,
    pub watcher_health: String,
    pub watcher_detail: String,
    pub updated_at_ms: i64,
}

impl Default for JobStatus {
    fn default() -> Self {
        Self {
            last_run_started_at_ms: None,
            last_run_finished_at_ms: None,
            last_outcome: None,
            watcher_health: "healthy".to_string(),
            watcher_detail: String::new(),
            updated_at_ms: now_ms(),
        }
    }
}

impl JobStatus {
    pub fn set_watcher_health(&mut self, health: WatchHealth, detail: &str) {
        self.watcher_health = match health {
            WatchHealth::Healthy => "healthy",
            WatchHealth::Degraded => "degraded",
            WatchHealth::Unavailable => "unavailable",
        }
        .to_string();
        self.watcher_detail = detail.to_string();
        self.updated_at_ms = now_ms();
    }
}

static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone)]
pub struct StatusStore {
    dir: PathBuf,
}

impl StatusStore {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path_for(&self, name: &str) -> std::io::Result<PathBuf> {
        if let Some(reason) = crate::store::invalid_job_name(name) {
            return Err(std::io::Error::new(ErrorKind::InvalidInput, reason));
        }
        Ok(self.dir.join(format!("{name}.json")))
    }

    pub fn read(&self, name: &str) -> std::io::Result<Option<JobStatus>> {
        let path = self.path_for(name)?;
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content)
                .map(Some)
                .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn update(&self, name: &str, f: impl FnOnce(&mut JobStatus)) -> std::io::Result<()> {
        let mut status = match self.read(name) {
            Ok(found) => found.unwrap_or_default(),
            Err(e) if e.kind() == ErrorKind::InvalidData => JobStatus::default(),
            Err(e) => return Err(e),
        };
        f(&mut status);
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path_for(name)?;
        let payload = serde_json::to_vec_pretty(&status)
            .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?;
        let counter = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!(
            "json.tmp.{}.{}.{counter}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&payload)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}
