// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::Write as _;
use std::path::{Path, PathBuf};

use cu_msg::Msg;

use crate::error::JobsError;
use crate::job::{AnomalyGuard, AutoUnlock, HookSpec, JobSpec, ScheduleWindow};

pub struct JobStore {
    path: PathBuf,
}

impl JobStore {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Vec<JobSpec>, JobsError> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        serde_json::from_str(&content)
            .map_err(|e| JobsError::Corrupt(self.path.clone(), e.to_string()))
    }

    pub fn add(&self, spec: JobSpec) -> Result<(), JobsError> {
        if let Some(reason) = invalid_job_name(&spec.name) {
            return Err(JobsError::InvalidName(reason));
        }
        self.with_lock(|mut jobs| {
            if jobs.iter().any(|j| j.name == spec.name) {
                return Err(JobsError::DuplicateName(spec.name));
            }
            jobs.push(spec);
            Ok((Some(jobs), ()))
        })
    }

    pub fn remove(&self, name: &str) -> Result<bool, JobsError> {
        self.with_lock(|mut jobs| {
            let before = jobs.len();
            jobs.retain(|j| j.name != name);
            let removed = jobs.len() != before;
            Ok((removed.then_some(jobs), removed))
        })
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<bool, JobsError> {
        self.update(name, |job| job.enabled = enabled)
    }

    pub fn set_auto_unlock(&self, name: &str, auto_unlock: AutoUnlock) -> Result<bool, JobsError> {
        self.update(name, |job| job.auto_unlock = auto_unlock)
    }

    pub fn set_excludes(&self, name: &str, excludes: Vec<String>) -> Result<bool, JobsError> {
        self.update(name, |job| job.excludes = excludes)
    }

    pub fn set_timing(
        &self,
        name: &str,
        timing: crate::job::TimingPrivacy,
    ) -> Result<bool, JobsError> {
        self.update(name, |job| job.timing = timing)
    }

    pub fn set_bandwidth_limit_kib(
        &self,
        name: &str,
        bandwidth_limit_kib: Option<u32>,
    ) -> Result<bool, JobsError> {
        self.update(name, |job| job.bandwidth_limit_kib = bandwidth_limit_kib)
    }

    pub fn set_schedule_windows(
        &self,
        name: &str,
        schedule_windows: Vec<ScheduleWindow>,
    ) -> Result<bool, JobsError> {
        self.update(name, |job| job.schedule_windows = schedule_windows)
    }

    pub fn set_pre_hook(&self, name: &str, pre_hook: Option<HookSpec>) -> Result<bool, JobsError> {
        self.update(name, |job| job.pre_hook = pre_hook)
    }

    pub fn set_post_hook(
        &self,
        name: &str,
        post_hook: Option<HookSpec>,
    ) -> Result<bool, JobsError> {
        self.update(name, |job| job.post_hook = post_hook)
    }

    pub fn set_anomaly_guard(
        &self,
        name: &str,
        anomaly_guard: AnomalyGuard,
    ) -> Result<bool, JobsError> {
        if let AnomalyGuard::Enabled(thresholds) = &anomaly_guard {
            thresholds
                .validate()
                .map_err(JobsError::InvalidThresholds)?;
        }
        self.update(name, |job| job.anomaly_guard = anomaly_guard)
    }

    fn update(&self, name: &str, f: impl FnOnce(&mut JobSpec)) -> Result<bool, JobsError> {
        self.with_lock(|mut jobs| {
            let Some(job) = jobs.iter_mut().find(|j| j.name == name) else {
                return Ok((None, false));
            };
            f(job);
            Ok((Some(jobs), true))
        })
    }

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("json.lock")
    }

    fn with_lock<T>(
        &self,
        f: impl FnOnce(Vec<JobSpec>) -> Result<(Option<Vec<JobSpec>>, T), JobsError>,
    ) -> Result<T, JobsError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_path())?;
        fs4::FileExt::lock(&lock_file)?;
        let jobs = self.load()?;
        let (to_save, result) = f(jobs)?;
        if let Some(jobs) = to_save {
            self.save(&jobs)?;
        }
        Ok(result)
    }

    fn save(&self, jobs: &[JobSpec]) -> Result<(), JobsError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let payload = serde_json::to_vec_pretty(jobs).map_err(|e| {
            JobsError::Corrupt(
                self.path.clone(),
                format!("cannot serialize the job list: {e}"),
            )
        })?;
        let tmp = tmp_path(&self.path);
        {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(&payload)?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut name = path
        .file_name()
        .map_or_else(|| "jobs.json".into(), std::ffi::OsStr::to_os_string);
    name.push(format!(".cryptoupload-tmp.{pid}.{nanos}.{counter}"));
    path.with_file_name(name)
}

#[must_use]
pub fn invalid_job_name(name: &str) -> Option<Msg> {
    if name.is_empty() {
        return Some(Msg::new("JOBS_NAME_EMPTY"));
    }
    if name == "." || name == ".." {
        return Some(Msg::new("JOBS_NAME_RESERVED").arg(name));
    }
    if name.contains(['/', '\\']) || name.contains('\0') {
        return Some(Msg::new("JOBS_NAME_CHARS").arg(name));
    }
    None
}
