// SPDX-License-Identifier: GPL-3.0-or-later

use cu_format::config::VaultConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryLockReport {
    pub locked: usize,
    pub failed: usize,
}

impl MemoryLockReport {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.failed == 0 && self.locked > 0
    }
}

pub(crate) struct MemoryLocks {
    _guards: Vec<region::LockGuard>,
    report: MemoryLockReport,
}

impl MemoryLocks {
    pub(crate) fn of(config: &VaultConfig) -> Self {
        let mut guards = Vec::new();
        let mut report = MemoryLockReport::default();
        config.for_each_long_lived_secret(&mut |bytes| match region::lock(
            bytes.as_ptr(),
            bytes.len(),
        ) {
            Ok(guard) => {
                guards.push(guard);
                report.locked += 1;
            }
            Err(_) => report.failed += 1,
        });
        Self {
            _guards: guards,
            report,
        }
    }

    pub(crate) fn report(&self) -> MemoryLockReport {
        self.report
    }
}
