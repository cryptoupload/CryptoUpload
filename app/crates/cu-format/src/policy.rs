// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::FormatError;

pub const POLICY_FIELD: &str = "org.cryptoupload.policy";

pub const POLICY_VERSION: u32 = 1;

pub const ELEVATED_MAX_IDLE_MINUTES: u32 = 5;

pub const ELEVATED_MASS_READ_THRESHOLD: u32 = 200;

pub const MAX_PROGRAMS: usize = 256;

pub const MAX_PROGRAM_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProgramAccess {
    #[default]
    Off,
    Allowlist,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct VaultPolicy {
    pub elevated: bool,
    pub lock_unmounts_drive: bool,
    pub lock_on_session_events: bool,
    pub max_idle_minutes: u32,
    pub block_remote_sessions: bool,
    pub program_access: ProgramAccess,
    pub allowed_programs: Vec<String>,
    pub read_only_drive: bool,
    pub no_unlock_shortcuts: bool,
    pub encrypt_local_state: bool,
    pub disable_activity_log: bool,
    pub decoys: bool,
    pub mass_read_threshold: u32,
    pub exempt_programs: Vec<String>,
}

impl Default for VaultPolicy {
    fn default() -> Self {
        Self {
            elevated: false,
            lock_unmounts_drive: false,
            lock_on_session_events: false,
            max_idle_minutes: 0,
            block_remote_sessions: false,
            program_access: ProgramAccess::Off,
            allowed_programs: Vec::new(),
            read_only_drive: false,
            no_unlock_shortcuts: false,
            encrypt_local_state: false,
            disable_activity_log: false,
            decoys: false,
            mass_read_threshold: 0,
            exempt_programs: Vec::new(),
        }
    }
}

impl VaultPolicy {
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    #[must_use]
    pub fn effective(&self) -> Self {
        if !self.elevated {
            return self.clone();
        }
        let mut out = self.clone();
        out.lock_unmounts_drive = true;
        out.lock_on_session_events = true;
        out.max_idle_minutes = tighter_cap(self.max_idle_minutes, ELEVATED_MAX_IDLE_MINUTES);
        out.block_remote_sessions = true;
        out.read_only_drive = true;
        out.no_unlock_shortcuts = true;
        out.decoys = true;
        if out.mass_read_threshold == 0 {
            out.mass_read_threshold = ELEVATED_MASS_READ_THRESHOLD;
        }
        out
    }

    #[must_use]
    pub fn weakened_by(&self, new: &Self) -> bool {
        let old = self.effective();
        let new = new.effective();
        let switched_off = [
            (old.elevated, new.elevated),
            (old.lock_unmounts_drive, new.lock_unmounts_drive),
            (old.lock_on_session_events, new.lock_on_session_events),
            (old.block_remote_sessions, new.block_remote_sessions),
            (old.read_only_drive, new.read_only_drive),
            (old.no_unlock_shortcuts, new.no_unlock_shortcuts),
            (old.encrypt_local_state, new.encrypt_local_state),
            (old.disable_activity_log, new.disable_activity_log),
            (old.decoys, new.decoys),
        ]
        .iter()
        .any(|&(before, after)| before && !after);

        switched_off
            || cap_loosened(old.max_idle_minutes, new.max_idle_minutes)
            || cap_loosened(old.mass_read_threshold, new.mass_read_threshold)
            || new.program_access < old.program_access
            || (old.program_access != ProgramAccess::Off
                && has_additions(&old.allowed_programs, &new.allowed_programs))
            || ((old.decoys || old.mass_read_threshold != 0)
                && has_additions(&old.exempt_programs, &new.exempt_programs))
    }

    pub fn from_value(value: &Value) -> Result<Self, FormatError> {
        let version = value
            .get("version")
            .and_then(Value::as_u64)
            .ok_or(FormatError::Malformed("policy without version"))?;
        let version = u32::try_from(version).unwrap_or(u32::MAX);
        if version != POLICY_VERSION {
            return Err(FormatError::UnsupportedPolicyVersion(version));
        }
        let mut fields = value.clone();
        if let Some(map) = fields.as_object_mut() {
            map.remove("version");
        }
        let policy: Self = serde_json::from_value(fields)
            .map_err(|_| FormatError::Malformed("policy not valid"))?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn to_value(&self) -> Result<Value, FormatError> {
        self.validate()?;
        let mut value = serde_json::to_value(self)
            .map_err(|_| FormatError::Malformed("policy cannot be serialized"))?;
        value
            .as_object_mut()
            .ok_or(FormatError::Malformed("policy cannot be serialized"))?
            .insert("version".to_string(), Value::from(POLICY_VERSION));
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), FormatError> {
        for list in [&self.allowed_programs, &self.exempt_programs] {
            if list.len() > MAX_PROGRAMS {
                return Err(FormatError::Malformed("program list too long"));
            }
            if list
                .iter()
                .any(|entry| entry.is_empty() || entry.len() > MAX_PROGRAM_BYTES)
            {
                return Err(FormatError::Malformed("program entry not valid"));
            }
        }
        Ok(())
    }
}

fn tighter_cap(current: u32, ceiling: u32) -> u32 {
    if current == 0 {
        ceiling
    } else {
        current.min(ceiling)
    }
}

fn cap_loosened(old: u32, new: u32) -> bool {
    old != 0 && (new == 0 || new > old)
}

fn has_additions(old: &[String], new: &[String]) -> bool {
    new.iter().any(|entry| !old.contains(entry))
}
