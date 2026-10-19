// SPDX-License-Identifier: GPL-3.0-or-later

pub struct Migration {
    pub version: i64,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../migrations/0002_multipart.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("../migrations/0003_prune.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("../migrations/0004_manifest.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("../migrations/0005_header_nonce.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("../migrations/0006_names_nfc.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("../migrations/0007_run_modified.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("../migrations/0008_sealed_names.sql"),
    },
];

#[must_use]
pub fn current_version() -> i64 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}
