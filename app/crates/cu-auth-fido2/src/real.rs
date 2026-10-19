// SPDX-License-Identifier: GPL-3.0-or-later

use ctap_hid_fido2::fidokey::{
    AssertionExtension, CredentialExtension, GetAssertionArgsBuilder, MakeCredentialArgsBuilder,
};
use ctap_hid_fido2::{Cfg, FidoKeyHidFactory, verifier};

use crate::{Fido2Authenticator, Fido2Error};

pub struct RealFido2Authenticator {
    cfg: Cfg,
}

impl RealFido2Authenticator {
    #[must_use]
    pub fn new() -> Self {
        Self { cfg: Cfg::init() }
    }

    fn open_device(&self) -> Result<ctap_hid_fido2::FidoKeyHid, Fido2Error> {
        let device = FidoKeyHidFactory::create(&self.cfg).map_err(|e| classify(&e.to_string()))?;
        let only_v2 = device.get_info().is_ok_and(|info| {
            !info.pin_uv_auth_protocols.contains(&1) && info.pin_uv_auth_protocols.contains(&2)
        });
        Ok(if only_v2 {
            device.with_pin_protocol_version(2)
        } else {
            device
        })
    }
}

fn classify(message: &str) -> Fido2Error {
    const PIN_ERRORS: [&str; 4] = [
        "CTAP2_ERR_PIN_REQUIRED",
        "CTAP2_ERR_PIN_INVALID",
        "CTAP2_ERR_PIN_AUTH_INVALID",
        "CTAP2_ERR_PIN_NOT_SET",
    ];
    if PIN_ERRORS.iter().any(|code| message.contains(code)) {
        Fido2Error::PinRequired
    } else if message.contains("FIDO device not found") {
        Fido2Error::NoDevice
    } else {
        Fido2Error::Other(message.to_string())
    }
}

impl Default for RealFido2Authenticator {
    fn default() -> Self {
        Self::new()
    }
}

impl Fido2Authenticator for RealFido2Authenticator {
    fn register(
        &self,
        rp_id: &str,
        salt: &[u8; 32],
        pin: Option<&str>,
    ) -> Result<(Vec<u8>, [u8; 32]), Fido2Error> {
        let device = self.open_device()?;
        let challenge = verifier::create_challenge();
        let mut builder = MakeCredentialArgsBuilder::new(rp_id, &challenge)
            .extensions(&[CredentialExtension::HmacSecret(Some(true))]);
        builder = match pin {
            Some(pin) => builder.pin(pin),
            None => builder.without_pin_and_uv(),
        };
        let attestation = device
            .make_credential_with_args(&builder.build())
            .map_err(|e| classify(&e.to_string()))?;

        let negotiated = attestation
            .extensions
            .iter()
            .any(|e| matches!(e, CredentialExtension::HmacSecret(Some(true))));
        if !negotiated {
            return Err(Fido2Error::HmacSecretUnsupported);
        }

        let credential_id = attestation.credential_descriptor.id;
        let secret = self.authenticate(rp_id, &credential_id, salt, pin)?;
        Ok((credential_id, secret))
    }

    fn authenticate(
        &self,
        rp_id: &str,
        credential_id: &[u8],
        salt: &[u8; 32],
        pin: Option<&str>,
    ) -> Result<[u8; 32], Fido2Error> {
        let device = self.open_device()?;
        let challenge = verifier::create_challenge();
        let mut builder = GetAssertionArgsBuilder::new(rp_id, &challenge)
            .credential_id(credential_id)
            .extensions(&[AssertionExtension::HmacSecret(Some(*salt))]);
        builder = match pin {
            Some(pin) => builder.pin(pin),
            None => builder.without_pin_and_uv(),
        };
        let assertions = device
            .get_assertion_with_args(&builder.build())
            .map_err(|e| classify(&e.to_string()))?;
        let secret = assertions.iter().find_map(|assertion| {
            assertion.extensions.iter().find_map(|e| match e {
                AssertionExtension::HmacSecret(Some(secret)) => Some(*secret),
                _ => None,
            })
        });
        secret.ok_or(Fido2Error::HmacSecretUnsupported)
    }
}
