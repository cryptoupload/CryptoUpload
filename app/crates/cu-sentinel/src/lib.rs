// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};

#[cfg(target_os = "linux")]
pub mod logind;
pub mod remote;

pub use remote::{RemoteEvidence, remote_control_evidence};

static SLEEP_REASON: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

pub fn set_sleep_reason(text: String) {
    *SLEEP_REASON
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(text);
}

#[cfg(target_os = "linux")]
fn sleep_reason() -> String {
    SLEEP_REASON
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap_or_else(|| "Locking the vault before sleep".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    Suspending,
    Resumed,
    ScreenLocked,
    SessionInactive,
}

#[derive(Debug)]
pub enum SentinelError {
    Unsupported,
    Unavailable(String),
}

impl Coded for SentinelError {
    fn msg(&self) -> Msg {
        match self {
            Self::Unsupported => Msg::new("SENTINEL_UNSUPPORTED"),
            Self::Unavailable(why) => Msg::new("SENTINEL_UNAVAILABLE").arg(why),
        }
    }
}

display_as_msg!(SentinelError);

impl std::error::Error for SentinelError {}

#[must_use]
pub const fn session_events_supported() -> bool {
    cfg!(target_os = "linux")
}

#[derive(Debug)]
pub struct SessionWatcher(());

pub fn watch_session(
    on_event: impl Fn(SessionEvent) + Send + Sync + 'static,
) -> Result<SessionWatcher, SentinelError> {
    #[cfg(target_os = "linux")]
    {
        let connection = zbus::blocking::Connection::system()
            .map_err(|e| SentinelError::Unavailable(e.to_string()))?;
        logind::watch(&connection, std::sync::Arc::new(on_event))?;
        Ok(SessionWatcher(()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = on_event;
        Err(SentinelError::Unsupported)
    }
}
