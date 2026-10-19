// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;
use std::time::{Duration, SystemTime};

pub const STALE_SPOOL_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[must_use]
pub fn sweep_stale(dir: &Path, prefix: &str, suffix: &str, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !(name.starts_with(prefix) && name.ends_with(suffix)) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let old = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > max_age);
        if old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}
