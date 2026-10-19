// SPDX-License-Identifier: GPL-3.0-or-later

use cu_format::config::VaultConfig;
use cu_vfs::VaultFs;

use crate::error::SyncError;

const MAX_ATTEMPTS: u32 = 50;

pub(crate) async fn sidecar_path(
    vault_fs: &VaultFs,
    config: &VaultConfig,
    vault_path: &str,
    device_label: Option<&str>,
    now_ms: i64,
) -> Result<String, SyncError> {
    let (dir, filename) = match vault_path.rsplit_once('/') {
        Some((d, f)) => (format!("{d}/"), f),
        None => (String::new(), vault_path),
    };
    let (stem, ext) = match filename.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, Some(e)),
        _ => (filename, None),
    };
    let when = format_utc_ms(now_ms);
    let tag = device_label.map_or_else(
        || format!("conflict {when}"),
        |label| format!("conflict {label} {when}"),
    );
    for attempt in 1..=MAX_ATTEMPTS {
        let suffix = if attempt == 1 {
            String::new()
        } else {
            format!(" ({attempt})")
        };
        let candidate_name = match ext {
            Some(e) => format!("{stem} ({tag}){suffix}.{e}"),
            None => format!("{stem} ({tag}){suffix}"),
        };
        let candidate = format!("{dir}{candidate_name}");
        if !vault_fs.exists(config, &candidate).await? {
            return Ok(candidate);
        }
    }
    Err(SyncError::IntegrityFailure(
        cu_msg::Msg::new("SYNC_NO_FREE_NAME")
            .arg(vault_path)
            .arg(MAX_ATTEMPTS),
    ))
}

#[must_use]
pub fn conflict_original_name(name: &str) -> Option<String> {
    let open = name.find(" (conflict ")?;
    let stem = &name[..open];
    let rest = &name[open + " (conflict ".len()..];
    let close = rest.find(')')?;
    let tag = &rest[..close];
    let when = tag.get(tag.len().checked_sub(WHEN_LEN)?..)?;
    if !is_utc_stamp(when) || (tag.len() > WHEN_LEN && !tag[..tag.len() - WHEN_LEN].ends_with(' '))
    {
        return None;
    }
    let mut tail = &rest[close + 1..];
    if let Some(after) = tail.strip_prefix(" (") {
        let end = after.find(')')?;
        if after[..end].is_empty() || !after[..end].bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        tail = &after[end + 1..];
    }
    if stem.is_empty() {
        return None;
    }
    match tail {
        "" => Some(stem.to_string()),
        t if t.starts_with('.') && t.len() > 1 => Some(format!("{stem}{t}")),
        _ => None,
    }
}

const WHEN_LEN: usize = 23;

fn is_utc_stamp(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == WHEN_LEN
        && s.ends_with(" UTC")
        && b.iter().enumerate().take(19).all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b' ',
            13 | 16 => *c == b'.',
            _ => c.is_ascii_digit(),
        })
}

fn format_utc_ms(ms: i64) -> String {
    let total_seconds = ms.div_euclid(1000);
    let days = total_seconds.div_euclid(86400);
    let secs_of_day = total_seconds.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02} {hh:02}.{mm:02}.{ss:02} UTC")
}

#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
