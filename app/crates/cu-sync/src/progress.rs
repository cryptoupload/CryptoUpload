// SPDX-License-Identifier: GPL-3.0-or-later

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOp {
    SetAside,
    Mkdir,
    Upload,
    Rename,
    Delete,
}

#[derive(Debug, Clone)]
pub enum SyncProgress {
    Item {
        vault_path: String,
        op: SyncOp,
        index: usize,
        total: usize,
    },
    Bytes {
        count: u64,
    },
    Plan {
        items: usize,
        upload_bytes: u64,
    },
}

#[derive(Debug, Clone, Default)]
pub struct CancelToken(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CancelToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}
