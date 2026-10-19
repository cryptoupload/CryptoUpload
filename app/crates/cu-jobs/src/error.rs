// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};
use std::path::PathBuf;

#[derive(Debug)]
pub enum JobsError {
    Io(std::io::Error),
    Corrupt(PathBuf, String),
    DuplicateName(String),
    InvalidName(Msg),
    InvalidThresholds(Msg),
}

impl Coded for JobsError {
    fn msg(&self) -> Msg {
        match self {
            Self::Io(e) => Msg::new("JOBS_IO").arg(e),
            Self::Corrupt(path, why) => Msg::new("JOBS_CORRUPT").arg(path.display()).arg(why),
            Self::DuplicateName(name) => Msg::new("JOBS_DUPLICATE_NAME").arg(name),
            Self::InvalidName(msg) | Self::InvalidThresholds(msg) => msg.clone(),
        }
    }
}

display_as_msg!(JobsError);

impl std::error::Error for JobsError {}

impl From<std::io::Error> for JobsError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
