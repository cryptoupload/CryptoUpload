// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::{mpsc, watch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchHealth {
    Healthy,
    Degraded,
    Unavailable,
}

pub struct DirWatcher {
    pub(crate) _watcher: Option<RecommendedWatcher>,
    pub wake: mpsc::Receiver<()>,
    pub health: watch::Receiver<(WatchHealth, String)>,
}

#[must_use]
pub fn spawn(path: &Path) -> DirWatcher {
    let (wake_tx, wake_rx) = mpsc::channel::<()>(1);
    let (health_tx, health_rx) = watch::channel((WatchHealth::Healthy, String::new()));

    let health_tx_for_events = health_tx.clone();
    let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| match res {
        Ok(event) => {
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            let _ = wake_tx.try_send(());
        }
        Err(e) => {
            let _ = health_tx_for_events.send((WatchHealth::Degraded, e.to_string()));
        }
    });

    let mut watcher = match watcher {
        Ok(w) => w,
        Err(e) => {
            let _ = health_tx.send((WatchHealth::Unavailable, e.to_string()));
            return DirWatcher {
                _watcher: None,
                wake: wake_rx,
                health: health_rx,
            };
        }
    };

    if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
        let _ = health_tx.send((WatchHealth::Unavailable, e.to_string()));
        return DirWatcher {
            _watcher: None,
            wake: wake_rx,
            health: health_rx,
        };
    }

    DirWatcher {
        _watcher: Some(watcher),
        wake: wake_rx,
        health: health_rx,
    }
}
