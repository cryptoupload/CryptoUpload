// SPDX-License-Identifier: GPL-3.0-or-later

mod real;

pub use real::RealFido2Authenticator;

use cu_msg::{Coded, Msg, display_as_msg};

#[derive(Debug)]
pub enum Fido2Error {
    NoDevice,
    HmacSecretUnsupported,
    PinRequired,
    Other(String),
}

impl Coded for Fido2Error {
    fn msg(&self) -> Msg {
        match self {
            Self::NoDevice => Msg::new("FIDO2_NO_DEVICE"),
            Self::HmacSecretUnsupported => Msg::new("FIDO2_NO_HMAC_SECRET"),
            Self::PinRequired => Msg::new("FIDO2_PIN_REQUIRED"),
            Self::Other(why) => Msg::new("FIDO2_OTHER").arg(why),
        }
    }
}

display_as_msg!(Fido2Error);

impl std::error::Error for Fido2Error {}

pub trait Fido2Authenticator: Send + Sync {
    fn register(
        &self,
        rp_id: &str,
        salt: &[u8; 32],
        pin: Option<&str>,
    ) -> Result<(Vec<u8>, [u8; 32]), Fido2Error>;

    fn authenticate(
        &self,
        rp_id: &str,
        credential_id: &[u8],
        salt: &[u8; 32],
        pin: Option<&str>,
    ) -> Result<[u8; 32], Fido2Error>;
}
