// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::AsyncReadExt;
use tokio::process::{ChildStderr, Command};
use tokio::time::Duration;

const STDERR_TAIL_MAX_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookOutcome {
    Ok,
    Failed {
        exit_code: Option<i32>,
        stderr_tail: String,
    },
    TimedOut,
    SpawnFailed(String),
}

impl HookOutcome {
    #[must_use]
    pub fn post_hook_msg(&self) -> cu_msg::Msg {
        use cu_msg::Msg;
        match self {
            Self::Ok => Msg::new("SYNC_POST_HOOK_OK"),
            Self::Failed {
                exit_code: Some(code),
                stderr_tail,
            } => Msg::new("SYNC_POST_HOOK_EXIT").arg(code).arg(stderr_tail),
            Self::Failed {
                exit_code: None,
                stderr_tail,
            } => Msg::new("SYNC_POST_HOOK_SIGNAL").arg(stderr_tail),
            Self::TimedOut => Msg::new("SYNC_POST_HOOK_TIMEOUT"),
            Self::SpawnFailed(reason) => Msg::new("SYNC_POST_HOOK_SPAWN").arg(reason),
        }
    }
}

const STDERR_DRAIN_GRACE: Duration = Duration::from_secs(2);

async fn drain_capped(mut pipe: ChildStderr, cap: usize) -> String {
    let mut buf = Vec::with_capacity(cap.min(4096));
    let mut chunk = [0u8; 4096];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if buf.len() < cap {
                    let take = (cap - buf.len()).min(n);
                    buf.extend_from_slice(&chunk[..take]);
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

pub async fn run_hook(hook: &HookSpec, env_vars: &[(&str, String)]) -> HookOutcome {
    let mut cmd = Command::new(&hook.program);
    cmd.args(&hook.args);
    cmd.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    #[cfg(windows)]
    if let Ok(user_profile) = std::env::var("USERPROFILE") {
        cmd.env("USERPROFILE", user_profile);
    }
    for (key, value) in env_vars {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return HookOutcome::SpawnFailed(e.to_string()),
    };

    let stderr_pipe = child.stderr.take();
    let mut stderr_task = tokio::spawn(async move {
        match stderr_pipe {
            Some(pipe) => drain_capped(pipe, STDERR_TAIL_MAX_BYTES).await,
            None => String::new(),
        }
    });

    let timeout = Duration::from_secs(hook.timeout_secs.max(1));
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => {
            let stderr_tail = match tokio::time::timeout(STDERR_DRAIN_GRACE, &mut stderr_task).await
            {
                Ok(joined) => joined.unwrap_or_default(),
                Err(_elapsed) => {
                    stderr_task.abort();
                    String::new()
                }
            };
            if status.success() {
                HookOutcome::Ok
            } else {
                HookOutcome::Failed {
                    exit_code: status.code(),
                    stderr_tail,
                }
            }
        }
        Ok(Err(e)) => HookOutcome::Failed {
            exit_code: None,
            stderr_tail: e.to_string(),
        },
        Err(_elapsed) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            HookOutcome::TimedOut
        }
    }
}
