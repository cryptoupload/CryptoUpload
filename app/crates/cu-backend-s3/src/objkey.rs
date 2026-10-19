// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::ObjPath;

#[must_use]
pub fn object_key(prefix: &str, path: &ObjPath) -> String {
    if prefix.is_empty() {
        path.as_str().to_string()
    } else if path.is_root() {
        prefix.to_string()
    } else {
        format!("{prefix}/{}", path.as_str())
    }
}

#[must_use]
pub fn list_prefix(prefix: &str, dir: &ObjPath) -> String {
    let key = object_key(prefix, dir);
    if key.is_empty() {
        String::new()
    } else {
        format!("{key}/")
    }
}

#[must_use]
pub fn normalize_prefix(prefix: &str) -> String {
    prefix.trim_matches('/').to_string()
}

#[must_use]
pub fn last_segment(key: &str) -> &str {
    key.trim_end_matches('/').rsplit('/').next().unwrap_or(key)
}
