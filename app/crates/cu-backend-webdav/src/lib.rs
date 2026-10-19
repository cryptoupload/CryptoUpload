// SPDX-License-Identifier: GPL-3.0-or-later

mod backend;
mod config;
mod errclass;
mod reader;
mod sweep;

mod writer;
mod xml;

pub use backend::WebDavBackend;
pub use config::WebDavConfig;
