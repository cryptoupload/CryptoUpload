// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;

use cu_index::NodeRow;

use crate::model::Plan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunStats {
    pub uploads: u64,
    pub deletes: u64,
    pub renames: u64,
    pub modified: Option<u64>,
}

impl From<&cu_index::RunRow> for RunStats {
    fn from(row: &cu_index::RunRow) -> Self {
        Self {
            uploads: row.files_uploaded,
            deletes: row.files_deleted,
            renames: row.files_renamed,
            modified: row.files_modified,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnomalyThresholds {
    pub mass_modify_abs: u64,
    pub mass_modify_multiplier: u64,
    pub mass_delete_abs: u64,
    pub mass_delete_multiplier: u64,
    pub rename_min_count: usize,
    pub rename_uniform_fraction_pct: u8,
    pub min_baseline_runs: usize,
}

impl Default for AnomalyThresholds {
    fn default() -> Self {
        Self {
            mass_modify_abs: 200,
            mass_modify_multiplier: 5,
            mass_delete_abs: 100,
            mass_delete_multiplier: 5,
            rename_min_count: 20,
            rename_uniform_fraction_pct: 80,
            min_baseline_runs: 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyGuard {
    Disabled,
    Enabled(AnomalyThresholds),
}

impl Default for AnomalyGuard {
    fn default() -> Self {
        Self::Enabled(AnomalyThresholds::default())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnomalyReport {
    pub reason: Vec<cu_msg::Msg>,
    pub uploads_affected: usize,
    pub deletes_affected: usize,
    pub renames_affected: usize,
}

fn extension_of(name: &str) -> Option<String> {
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    let ext = &name[dot + 1..];
    if ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

#[must_use]
pub fn known_extensions(nodes: &[NodeRow]) -> HashSet<String> {
    nodes.iter().filter_map(|n| extension_of(&n.name)).collect()
}

fn median_u64(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        u64::midpoint(sorted[mid - 1], sorted[mid])
    } else {
        sorted[mid]
    }
}

fn exceeds(
    count: u64,
    abs_threshold: u64,
    multiplier: u64,
    baseline: &[u64],
    min_runs: usize,
) -> bool {
    if count == 0 || count < abs_threshold {
        return false;
    }
    if baseline.len() < min_runs {
        return true;
    }
    let median = median_u64(baseline);
    count >= median.saturating_mul(multiplier)
}

#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn detect(
    plan: &Plan,
    baseline: &[RunStats],
    known_extensions: &HashSet<String>,
    thresholds: &AnomalyThresholds,
) -> Option<AnomalyReport> {
    let uploads_affected = plan
        .uploads
        .iter()
        .filter(|u| u.archive_previous.is_some())
        .count();
    let deletes_affected = plan.deletes.len();
    let total_renames = plan.renames.len();

    let known_modified: Vec<u64> = baseline.iter().filter_map(|r| r.modified).collect();
    let modify_baseline: Vec<u64> = if known_modified.len() >= thresholds.min_baseline_runs {
        known_modified
    } else {
        baseline.iter().map(|r| r.uploads).collect()
    };
    let mass_modify = exceeds(
        uploads_affected as u64,
        thresholds.mass_modify_abs,
        thresholds.mass_modify_multiplier,
        &modify_baseline,
        thresholds.min_baseline_runs,
    );

    let delete_baseline: Vec<u64> = baseline.iter().map(|r| r.deletes).collect();
    let mass_delete = exceeds(
        deletes_affected as u64,
        thresholds.mass_delete_abs,
        thresholds.mass_delete_multiplier,
        &delete_baseline,
        thresholds.min_baseline_runs,
    );

    let uniform_rename = if total_renames >= thresholds.rename_min_count {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for r in &plan.renames {
            if let Some(ext) = extension_of(&r.new_name) {
                *counts.entry(ext).or_insert(0) += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .filter(|(ext, count)| {
                let uniform_enough = count.saturating_mul(100)
                    >= usize::from(thresholds.rename_uniform_fraction_pct) * total_renames;
                uniform_enough && !known_extensions.contains(ext)
            })
    } else {
        None
    };

    let mut reasons = Vec::new();
    let mut renames_affected = 0;
    if let Some((ext, count)) = &uniform_rename {
        reasons.push(
            cu_msg::Msg::new("ANOMALY_RENAME")
                .arg(count)
                .arg(total_renames)
                .arg(ext),
        );
        renames_affected = *count;
    }
    if mass_modify {
        reasons.push(
            cu_msg::Msg::new("ANOMALY_MODIFY")
                .arg(uploads_affected)
                .arg(thresholds.mass_modify_abs)
                .arg(median_u64(&modify_baseline)),
        );
    }
    if mass_delete {
        reasons.push(
            cu_msg::Msg::new("ANOMALY_DELETE")
                .arg(deletes_affected)
                .arg(thresholds.mass_delete_abs)
                .arg(median_u64(&delete_baseline)),
        );
    }

    if reasons.is_empty() {
        return None;
    }
    Some(AnomalyReport {
        reason: reasons,
        uploads_affected,
        deletes_affected,
        renames_affected,
    })
}
