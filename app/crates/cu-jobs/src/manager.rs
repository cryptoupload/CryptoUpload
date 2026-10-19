// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use tokio::sync::watch;

use crate::job::{JobRunner, JobSpec};
use crate::scheduler::run_job;
use crate::status::StatusStore;

pub struct JobManager<R: JobRunner + 'static> {
    runner: Arc<R>,
    status: StatusStore,
}

impl<R: JobRunner + 'static> JobManager<R> {
    #[must_use]
    pub fn new(runner: R, status: StatusStore) -> Self {
        Self {
            runner: Arc::new(runner),
            status,
        }
    }

    pub async fn run(&self, jobs: Vec<JobSpec>) {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let run = self.run_until(jobs, shutdown_rx);
        tokio::pin!(run);

        tokio::select! {
            () = &mut run => {}
            () = wait_for_shutdown_signal() => {
                let _ = shutdown_tx.send(true);
                run.await;
            }
        }
    }

    pub async fn run_until(&self, jobs: Vec<JobSpec>, shutdown: watch::Receiver<bool>) {
        let handles: Vec<_> = jobs
            .into_iter()
            .filter(|j| j.enabled)
            .map(|spec| {
                tokio::spawn(run_job(
                    spec,
                    self.runner.clone(),
                    self.status.clone(),
                    shutdown.clone(),
                ))
            })
            .collect();

        for handle in handles {
            let _ = handle.await;
        }
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler registration");
    let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler registration");
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
