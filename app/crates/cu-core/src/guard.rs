// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::{Component, Path};

use crate::error::CoreError;

const EXACT_MARKERS: &[&str] = &["Dropbox", "Google Drive", "CloudStorage"];
const PREFIX_MARKERS: &[&str] = &["OneDrive"];

pub fn check_not_inside_third_party_sync(root: &Path) -> Result<(), CoreError> {
    for component in root.components() {
        let Component::Normal(os_str) = component else {
            continue;
        };
        let Some(name) = os_str.to_str() else {
            continue;
        };
        let matched = EXACT_MARKERS.contains(&name)
            || PREFIX_MARKERS.iter().any(|prefix| name.starts_with(prefix));
        if matched {
            return Err(CoreError::GuardRejected(
                Msg::new("GUARD_THIRD_PARTY_SYNC")
                    .arg(root.display())
                    .arg(name),
            ));
        }
    }
    Ok(())
}
