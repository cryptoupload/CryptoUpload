// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::VecDeque;

use cu_backend::{BackendError, EntryKind, ObjPath, StorageBackend};

use crate::backend::WebDavBackend;
use crate::writer::TEMP_PREFIX;

impl WebDavBackend {
    pub async fn sweep_orphans(&self, cutoff_ms: i64) -> Result<Vec<String>, BackendError> {
        let mut deleted = Vec::new();
        let mut queue = VecDeque::from([ObjPath::root()]);
        while let Some(dir) = queue.pop_front() {
            for entry in self.list_including_temp(&dir).await? {
                let path = dir.join(&entry.name)?;
                match entry.kind {
                    EntryKind::Directory => queue.push_back(path),
                    EntryKind::File if entry.name.starts_with(TEMP_PREFIX) => {
                        let Some(modified_secs) = entry.modified else {
                            continue;
                        };
                        if modified_secs.saturating_mul(1000) >= cutoff_ms {
                            continue;
                        }
                        if self.delete(&path).await.is_ok() {
                            deleted.push(path.as_str().to_string());
                        }
                    }
                    EntryKind::File => {}
                }
            }
        }
        Ok(deleted)
    }
}
