// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::VecDeque;

use cu_sync::CancelToken;

use crate::error::CoreError;
use crate::vault::{NodeKind, Vault};

pub const DEFAULT_FIND_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundEntry {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindResult {
    pub entries: Vec<FoundEntry>,
    pub truncated: bool,
}

#[must_use]
pub fn name_matches(pattern: &str, name: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    let pattern: Vec<char> = cu_format::name::nfc(pattern)
        .to_lowercase()
        .chars()
        .collect();
    let name: Vec<char> = cu_format::name::nfc(name).to_lowercase().chars().collect();
    if pattern.iter().any(|c| matches!(c, '*' | '?')) {
        glob_matches(&pattern, &name)
    } else {
        name.windows(pattern.len()).any(|w| w == pattern.as_slice())
    }
}

fn glob_matches(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || Some(&c) == name.get(n) => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((star_p, star_n)) => {
                    p = star_p + 1;
                    n = star_n + 1;
                    star = Some((star_p, star_n + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FindOutcome {
    pub truncated: bool,
    pub cancelled: bool,
}

impl Vault {
    pub async fn find(
        &self,
        start: &str,
        pattern: &str,
        limit: usize,
    ) -> Result<FindResult, CoreError> {
        let mut entries = Vec::new();
        let outcome = self
            .find_streaming(start, pattern, limit, &CancelToken::new(), &mut |found| {
                entries.push(found);
            })
            .await?;
        Ok(FindResult {
            entries,
            truncated: outcome.truncated,
        })
    }

    pub async fn find_streaming(
        &self,
        start: &str,
        pattern: &str,
        limit: usize,
        cancel: &CancelToken,
        on_found: &mut (dyn FnMut(FoundEntry) + Send),
    ) -> Result<FindOutcome, CoreError> {
        let mut outcome = FindOutcome::default();
        if pattern.is_empty() || limit == 0 {
            return Ok(outcome);
        }
        let fs = self.fs();
        let config = self.config();
        let start_prefix = start.trim_matches('/').to_string();
        let root = fs.resolve_dir_ref(config, &start_prefix).await?;

        let mut delivered = 0usize;
        let mut queue = VecDeque::from([(start_prefix, root)]);
        while let Some((dir_path, dir_ref)) = queue.pop_front() {
            if cancel.is_cancelled() {
                outcome.cancelled = true;
                return Ok(outcome);
            }
            let entries = fs.list_in(config, &dir_ref).await?;
            for entry in entries {
                if dir_path.is_empty() && entry.name == ".cryptoupload" {
                    continue;
                }
                let is_dir = entry.kind == NodeKind::Dir;
                let full = if dir_path.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{dir_path}/{}", entry.name)
                };
                if name_matches(pattern, &entry.name) {
                    if delivered >= limit {
                        outcome.truncated = true;
                        return Ok(outcome);
                    }
                    delivered += 1;
                    on_found(FoundEntry {
                        path: full.clone(),
                        is_dir,
                    });
                }
                if is_dir {
                    let child = fs.descend_dir_ref(config, &dir_ref, &entry.name).await?;
                    queue.push_back((full, child));
                }
            }
        }
        Ok(outcome)
    }
}
