// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};

#[derive(Debug)]
pub enum IndexError {
    Sqlite(rusqlite::Error),
    SchemaTooNew { found: i64, known: i64 },
    Locked,
    SealBroken,
    SealFailed,
}

impl Coded for IndexError {
    fn msg(&self) -> Msg {
        match self {
            Self::Sqlite(e) => Msg::new("INDEX_SQLITE").arg(e),
            Self::SchemaTooNew { found, known } => {
                Msg::new("INDEX_SCHEMA_TOO_NEW").arg(found).arg(known)
            }
            Self::Locked => Msg::new("INDEX_LOCKED"),
            Self::SealBroken => Msg::new("INDEX_SEAL_BROKEN"),
            Self::SealFailed => Msg::new("INDEX_SEAL_FAILED"),
        }
    }
}

display_as_msg!(IndexError);

impl std::error::Error for IndexError {}

impl From<rusqlite::Error> for IndexError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}
