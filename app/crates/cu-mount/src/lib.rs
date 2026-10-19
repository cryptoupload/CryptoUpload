// SPDX-License-Identifier: GPL-3.0-or-later

pub mod cache;
pub mod inode;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod ops;
pub mod readahead;
pub mod sentry;
pub mod staging;
#[cfg(windows)]
pub mod windows;
pub mod write;

pub use ops::{Attr, Fh, OpError, VfsOps};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForcedUnmount {
    Severed,
    Detached,
}

#[cfg(target_os = "linux")]
pub use linux as platform;
#[cfg(windows)]
pub use windows as platform;
