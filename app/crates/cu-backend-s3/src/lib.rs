// SPDX-License-Identifier: GPL-3.0-or-later

mod backend;
mod config;
mod errclass;
mod objkey;
mod reader;
mod sweep;

mod writer;

pub use backend::S3Backend;
pub use config::{ObjectLockMode, S3Config, check_proxy_supported};
pub use writer::PART_SIZE;
