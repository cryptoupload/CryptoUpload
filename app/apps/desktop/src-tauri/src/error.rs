// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg};

#[derive(Debug)]
pub struct UiError(String);

impl UiError {
    #[must_use]
    pub fn no_vault() -> Self {
        Self::coded(&Msg::new("NO_VAULT_UNLOCKED"))
    }

    #[must_use]
    pub fn coded(msg: &Msg) -> Self {
        Self(msg.wire())
    }

    #[must_use]
    pub fn from_msg(err: &impl Coded) -> Self {
        Self::coded(&err.msg())
    }
}

pub const CONFIG_ROLLED_BACK: &str = "CONFIG_ROLLED_BACK";

impl From<cu_core::CoreError> for UiError {
    fn from(err: cu_core::CoreError) -> Self {
        match err {
            cu_core::CoreError::ConfigRolledBack { .. } => Self(CONFIG_ROLLED_BACK.to_string()),
            other => Self::from_msg(&other),
        }
    }
}

impl serde::Serialize for UiError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}
