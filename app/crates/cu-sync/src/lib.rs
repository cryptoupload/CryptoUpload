// SPDX-License-Identifier: GPL-3.0-or-later

pub mod anomaly;
mod conflict;
pub mod cuignore;
pub mod diff;
pub mod engine;
pub mod error;
pub mod execute;
mod hexutil;
pub mod hooks;
mod manifest;
mod manifest_log;
pub mod model;
pub mod progress;
pub mod registry;
pub mod registry_verify;
pub mod restore;
pub mod verify;

pub use conflict::conflict_original_name;
pub use cuignore::CuIgnore;
pub use engine::{SyncEngine, SyncReport};
pub use error::SyncError;
pub use execute::RetentionPolicy;
pub use model::{Conflict, ConflictKind, ConflictResolution, Plan, PlannedSetAside};
pub use progress::{CancelToken, SyncOp, SyncProgress};
