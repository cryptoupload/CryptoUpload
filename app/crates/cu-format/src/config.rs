// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use cu_crypto::Secret;
use cu_crypto::kdf::KdfParams;
use data_encoding::BASE64URL_NOPAD;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::common::{SPEC_VERSION, SeedId};
use crate::error::FormatError;
use crate::policy::{POLICY_FIELD, VaultPolicy};

pub const FILE_FORMAT: &str = "AES-256-GCM-32k";
pub const NAME_FORMAT: &str = "AES-SIV-512-B64URL";

pub const PRIVATE_FILE_FORMAT: &str = "org.cryptoupload.AES-256-GCM-32k-padme.v1";
pub const PRIVATE_NAME_FORMAT: &str = "org.cryptoupload.AES-SIV-512-B64URL-pad32.v1";
const MEMBER_NAMES_FIELD: &str = "org.cryptoupload.memberNames";
const DECOY_ENTRIES_FIELD: &str = "org.cryptoupload.decoyEntries";
const CONFIG_SEQ_FIELD: &str = "org.cryptoupload.configSeq";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatProfile {
    #[default]
    Standard,
    Private,
}

impl FormatProfile {
    #[must_use]
    pub const fn file_format(self) -> &'static str {
        match self {
            Self::Standard => FILE_FORMAT,
            Self::Private => PRIVATE_FILE_FORMAT,
        }
    }

    #[must_use]
    pub const fn name_format(self) -> &'static str {
        match self {
            Self::Standard => NAME_FORMAT,
            Self::Private => PRIVATE_NAME_FORMAT,
        }
    }
}
pub const KDF_NAME: &str = "HKDF-SHA512";

const PROTECTED_HEADER_B64URL: &str = "eyJlbmMiOiJBMjU2R0NNIiwiY3R5IjoianNvbiIsImNyaXQiOlsidXZmLnNwZWMudmVyc2lvbiJdLCJ1dmYuc3BlYy52ZXJzaW9uIjoxfQ";

const PASSWORD_KID: &str = "org.cryptoupload.password.v1";
const PASSWORD_ALG: &str = "A256KW";
const SEED_KID: &str = "org.cryptoupload.seed.v1";
const SEED_ALG: &str = "A256KW";
const SEED_KEK_INFO: &[u8] = b"org.cryptoupload.seed-kek.v1";

pub const FIDO2_KID: &str = "org.cryptoupload.fido2.v1";
const FIDO2_ALG: &str = "A256KW";
pub const TWO_FACTOR_KID: &str = "org.cryptoupload.password+fido2.v1";
const TWO_FACTOR_ALG: &str = "A256KW";
const TWO_FACTOR_KEK_INFO: &[u8] = b"org.cryptoupload.password+fido2-kek.v1";

const FIDO2_KEK_INFO: &[u8] = b"org.cryptoupload.fido2-kek.v1";

pub const PLATFORM_KID: &str = "org.cryptoupload.platform.v1";
const PLATFORM_ALG: &str = "A256KW";

pub const MEMBER_KID_PREFIX: &str = "org.cryptoupload.member.v1:";
const MEMBER_ALG: &str = "A256KW";
const MEMBER_KEK_BITS: u32 = 256;

pub enum Credential<'a> {
    Password(&'a [u8]),
    RecoverySeed(&'a Secret<32>),
    Fido2Secret(&'a Secret<32>),
    PasswordAndFido2 {
        password: &'a [u8],
        fido2_secret: &'a Secret<32>,
    },
    PlatformKey(&'a Secret<32>),
    MemberKey(&'a cu_crypto::x25519::StaticKeypair),
}

#[derive(Default)]
pub struct RekeyInputs<'a> {
    pub password: Option<&'a [u8]>,
    pub new_recovery_entropy: Option<&'a Secret<32>>,
    pub fido2_secret: Option<&'a Secret<32>>,
    pub platform_kek: Option<&'a Secret<32>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DroppedRecipient {
    Recovery,
    Fido2,
    Platform,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RekeyReport {
    pub dropped: Vec<DroppedRecipient>,
}

#[derive(Debug, Clone)]
pub struct VaultConfig {
    seeds: HashMap<SeedId, Secret<32>>,
    initial_seed_id: SeedId,
    latest_seed_id: SeedId,
    kdf_salt: [u8; 32],
    cek: Secret<32>,
    recipients: Vec<JweRecipient>,
    inbox_public_key: Option<[u8; 32]>,
    policy: VaultPolicy,
    profile: FormatProfile,
    member_names: std::collections::BTreeMap<String, String>,
    decoy_entries: bool,
    config_seq: u64,
    extra_payload_fields: Map<String, Value>,
}

impl VaultConfig {
    #[must_use]
    pub fn seed(&self, id: SeedId) -> Option<&Secret<32>> {
        self.seeds.get(&id)
    }

    #[must_use]
    pub const fn initial_seed_id(&self) -> SeedId {
        self.initial_seed_id
    }

    #[must_use]
    pub fn initial_seed(&self) -> &Secret<32> {
        #[allow(clippy::expect_used)]
        self.seeds
            .get(&self.initial_seed_id)
            .expect("initialSeed is always in the seeds map: checked in create/open")
    }

    #[must_use]
    pub const fn latest_seed_id(&self) -> SeedId {
        self.latest_seed_id
    }

    #[must_use]
    pub fn latest_seed(&self) -> &Secret<32> {
        #[allow(clippy::expect_used)]
        self.seeds
            .get(&self.latest_seed_id)
            .expect("latestSeed is always in the seeds map: checked in create/open")
    }

    #[must_use]
    pub fn seed_count(&self) -> usize {
        self.seeds.len()
    }

    #[must_use]
    pub const fn kdf_salt(&self) -> &[u8; 32] {
        &self.kdf_salt
    }

    #[must_use]
    pub fn has_recovery_recipient(&self) -> bool {
        find_recipient(&self.recipients, SEED_KID).is_some()
    }

    #[must_use]
    pub fn has_fido2_recipient(&self) -> bool {
        find_recipient(&self.recipients, FIDO2_KID).is_some()
    }

    #[must_use]
    pub fn has_platform_recipient(&self) -> bool {
        find_recipient(&self.recipients, PLATFORM_KID).is_some()
    }

    #[must_use]
    pub fn has_inbox(&self) -> bool {
        self.inbox_public_key.is_some()
    }

    #[must_use]
    pub fn inbox_public_key(&self) -> Option<[u8; 32]> {
        self.inbox_public_key
    }

    pub fn for_each_long_lived_secret(&self, visit: &mut dyn FnMut(&[u8])) {
        visit(self.cek.expose());
        for seed in self.seeds.values() {
            visit(seed.expose());
        }
    }

    #[must_use]
    pub const fn profile(&self) -> FormatProfile {
        self.profile
    }

    #[must_use]
    pub const fn decoy_entries(&self) -> bool {
        self.decoy_entries
    }

    pub fn set_decoy_entries(&mut self, enabled: bool) -> Result<(), FormatError> {
        if enabled && self.profile != FormatProfile::Private {
            return Err(FormatError::NotInThisProfile("decoys"));
        }
        self.decoy_entries = enabled;
        Ok(())
    }

    #[must_use]
    pub fn policy(&self) -> &VaultPolicy {
        &self.policy
    }

    #[must_use]
    pub const fn config_seq(&self) -> u64 {
        self.config_seq
    }

    pub const fn bump_config_seq(&mut self) {
        self.config_seq = self.config_seq.saturating_add(1);
    }

    pub fn set_policy(&mut self, policy: VaultPolicy) -> Result<(), FormatError> {
        policy.validate()?;
        self.policy = policy;
        Ok(())
    }

    pub fn inbox_keypair(&self) -> Result<cu_crypto::x25519::StaticKeypair, FormatError> {
        let seed = crate::keys::inbox_seed(self.initial_seed(), &self.kdf_salt)?;
        Ok(cu_crypto::x25519::StaticKeypair::from_private_bytes(
            *seed.expose(),
        ))
    }

    pub fn enable_inbox(&mut self) -> Result<[u8; 32], FormatError> {
        if self.profile == FormatProfile::Private {
            return Err(FormatError::NotInThisProfile("inbox"));
        }
        let public_key = self.inbox_keypair()?.public_bytes();
        self.inbox_public_key = Some(public_key);
        Ok(public_key)
    }

    pub fn disable_inbox(&mut self) {
        self.inbox_public_key = None;
    }

    pub fn recipients_info(&self) -> Result<Vec<RecipientInfo>, FormatError> {
        self.recipients
            .iter()
            .map(|recipient| {
                let mut info = recipient_info_of(recipient)?;
                if let RecipientInfo::Member { name, public_key } = &mut info
                    && let Some(private_name) = self.member_names.get(&member_kid(public_key))
                {
                    name.clone_from(private_name);
                }
                Ok(info)
            })
            .collect()
    }

    pub fn set_password(
        &mut self,
        password: &[u8],
        argon2_params: KdfParams,
    ) -> Result<(), FormatError> {
        let argon2_salt: [u8; 16] = random_array()?;
        let kek_pw = cu_crypto::kdf::derive_kek(password, &argon2_salt, argon2_params)?;
        let encrypted_key = cu_crypto::keywrap::wrap(&kek_pw, self.cek.expose())?;

        let header = PasswordRecipientHeader {
            alg: PASSWORD_ALG.to_string(),
            kid: PASSWORD_KID.to_string(),
            argon2_salt: b64(&argon2_salt),
            argon2_m: argon2_params.m_cost_kib(),
            argon2_t: argon2_params.t_cost(),
            argon2_p: argon2_params.p_cost(),
        };
        let recipient = JweRecipient {
            header: serde_json::to_value(&header)
                .map_err(|_| FormatError::Malformed("recipient cannot be serialized"))?,
            encrypted_key: b64(&encrypted_key),
        };
        replace_recipient(&mut self.recipients, PASSWORD_KID, recipient);
        Ok(())
    }

    pub fn set_recovery_seed(&mut self, entropy: &Secret<32>) -> Result<(), FormatError> {
        let seed_salt: [u8; 16] = random_array()?;
        let kek_seed =
            cu_crypto::hkdf::derive_secret(entropy.expose(), Some(&seed_salt), SEED_KEK_INFO)?;
        let encrypted_key = cu_crypto::keywrap::wrap(&kek_seed, self.cek.expose())?;

        let header = SeedRecipientHeader {
            alg: SEED_ALG.to_string(),
            kid: SEED_KID.to_string(),
            seed_salt: b64(&seed_salt),
        };
        let recipient = JweRecipient {
            header: serde_json::to_value(&header)
                .map_err(|_| FormatError::Malformed("recipient cannot be serialized"))?,
            encrypted_key: b64(&encrypted_key),
        };
        replace_recipient(&mut self.recipients, SEED_KID, recipient);
        Ok(())
    }

    pub fn set_fido2_recipient(
        &mut self,
        credential_id: &[u8],
        salt: &[u8; 32],
        rp_id: &str,
        hmac_secret: &Secret<32>,
    ) -> Result<(), FormatError> {
        let kek_fido2 = cu_crypto::hkdf::derive_secret(
            hmac_secret.expose(),
            Some(salt.as_slice()),
            FIDO2_KEK_INFO,
        )?;
        let encrypted_key = cu_crypto::keywrap::wrap(&kek_fido2, self.cek.expose())?;

        let header = Fido2RecipientHeader {
            alg: FIDO2_ALG.to_string(),
            kid: FIDO2_KID.to_string(),
            credential_id: b64(credential_id),
            salt: b64(salt),
            rp_id: rp_id.to_string(),
        };
        let recipient = JweRecipient {
            header: serde_json::to_value(&header)
                .map_err(|_| FormatError::Malformed("recipient cannot be serialized"))?,
            encrypted_key: b64(&encrypted_key),
        };
        replace_recipient(&mut self.recipients, FIDO2_KID, recipient);
        Ok(())
    }

    pub fn set_two_factor_recipient(
        &mut self,
        password: &[u8],
        argon2_params: KdfParams,
        credential_id: &[u8],
        salt: &[u8; 32],
        rp_id: &str,
        hmac_secret: &Secret<32>,
    ) -> Result<(), FormatError> {
        let argon2_salt: [u8; 16] = random_array()?;
        let kek = two_factor_kek(password, &argon2_salt, argon2_params, salt, hmac_secret)?;
        let encrypted_key = cu_crypto::keywrap::wrap(&kek, self.cek.expose())?;
        let header = TwoFactorRecipientHeader {
            alg: TWO_FACTOR_ALG.to_string(),
            kid: TWO_FACTOR_KID.to_string(),
            argon2_salt: b64(&argon2_salt),
            argon2_m: argon2_params.m_cost_kib(),
            argon2_t: argon2_params.t_cost(),
            argon2_p: argon2_params.p_cost(),
            credential_id: b64(credential_id),
            salt: b64(salt),
            rp_id: rp_id.to_string(),
        };
        let recipient = JweRecipient {
            header: serde_json::to_value(&header)
                .map_err(|_| FormatError::Malformed("recipient cannot be serialized"))?,
            encrypted_key: b64(&encrypted_key),
        };
        replace_recipient(&mut self.recipients, TWO_FACTOR_KID, recipient);
        self.recipients.retain(|r| {
            let kid = r.header.get("kid").and_then(Value::as_str);
            kid != Some(PASSWORD_KID) && kid != Some(FIDO2_KID)
        });
        Ok(())
    }

    #[must_use]
    pub fn has_two_factor_recipient(&self) -> bool {
        find_recipient(&self.recipients, TWO_FACTOR_KID).is_some()
    }

    pub fn remove_two_factor(
        &mut self,
        password: &[u8],
        argon2_params: KdfParams,
    ) -> Result<(), FormatError> {
        if !self.has_two_factor_recipient() {
            return Err(FormatError::NoSuchRecipient);
        }
        self.set_password(password, argon2_params)?;
        self.recipients
            .retain(|r| r.header.get("kid").and_then(Value::as_str) != Some(TWO_FACTOR_KID));
        Ok(())
    }

    pub fn set_platform_recipient(
        &mut self,
        keychain_id: &[u8; 16],
        kek: &Secret<32>,
    ) -> Result<(), FormatError> {
        let encrypted_key = cu_crypto::keywrap::wrap(kek, self.cek.expose())?;

        let header = PlatformRecipientHeader {
            alg: PLATFORM_ALG.to_string(),
            kid: PLATFORM_KID.to_string(),
            keychain_id: b64(keychain_id),
        };
        let recipient = JweRecipient {
            header: serde_json::to_value(&header)
                .map_err(|_| FormatError::Malformed("recipient cannot be serialized"))?,
            encrypted_key: b64(&encrypted_key),
        };
        replace_recipient(&mut self.recipients, PLATFORM_KID, recipient);
        Ok(())
    }

    pub fn set_member_recipient(
        &mut self,
        member_public_key: &[u8; 32],
        name: &str,
    ) -> Result<(), FormatError> {
        let (epk_public, z) = cu_crypto::x25519::ephemeral_diffie_hellman(member_public_key)
            .map_err(|e| match e {
                cu_crypto::CryptoError::RandomUnavailable => FormatError::RandomUnavailable,
                _ => FormatError::InvalidMemberPublicKey,
            })?;
        let kek_bytes = cu_crypto::concat_kdf::derive(
            z.expose(),
            MEMBER_KEK_BITS,
            MEMBER_ALG.as_bytes(),
            &epk_public,
            member_public_key,
        )?;
        let kek_array: [u8; 32] = kek_bytes
            .as_slice()
            .try_into()
            .map_err(|_| FormatError::Malformed("member KEK is not 32 bytes"))?;
        let kek = Secret::new(kek_array);
        let encrypted_key = cu_crypto::keywrap::wrap(&kek, self.cek.expose())?;

        let kid = member_kid(member_public_key);
        let header_name = match self.profile {
            FormatProfile::Standard => name.to_string(),
            FormatProfile::Private => {
                self.member_names.insert(kid.clone(), name.to_string());
                String::new()
            }
        };
        let header = MemberRecipientHeader {
            alg: MEMBER_ALG.to_string(),
            kid: kid.clone(),
            epk: b64(&epk_public),
            apu: b64(&epk_public),
            apv: b64(member_public_key),
            public_key: b64(member_public_key),
            name: header_name,
        };
        let recipient = JweRecipient {
            header: serde_json::to_value(&header)
                .map_err(|_| FormatError::Malformed("recipient cannot be serialized"))?,
            encrypted_key: b64(&encrypted_key),
        };
        replace_recipient(&mut self.recipients, &kid, recipient);
        Ok(())
    }

    pub fn remove_recipient_by_kid(&mut self, kid: &str) -> Result<(), FormatError> {
        if find_recipient(&self.recipients, kid).is_none() {
            return Err(FormatError::NoSuchRecipient);
        }
        if self.recipients.len() <= 1 {
            return Err(FormatError::LastRecipient);
        }
        self.recipients
            .retain(|r| r.header.get("kid").and_then(Value::as_str) != Some(kid));
        self.member_names.remove(kid);
        Ok(())
    }

    pub fn rotate_cek(&mut self, inputs: &RekeyInputs<'_>) -> Result<RekeyReport, FormatError> {
        let mut next = self.clone();
        next.cek = Secret::<32>::random()?;
        next.recipients.clear();
        let mut dropped = Vec::new();

        for recipient in &self.recipients {
            let kid = recipient
                .header
                .get("kid")
                .and_then(Value::as_str)
                .unwrap_or("");
            if kid.starts_with(MEMBER_KID_PREFIX) {
                let header: MemberRecipientHeader =
                    serde_json::from_value(recipient.header.clone()).map_err(|_| {
                        FormatError::Malformed("member recipient header incomplete")
                    })?;
                let public_key: [u8; 32] = decode_b64(&header.public_key)?
                    .try_into()
                    .map_err(|_| FormatError::Malformed("member public key is not 32 bytes"))?;
                let name = self.member_names.get(kid).unwrap_or(&header.name);
                next.set_member_recipient(&public_key, name)?;
                continue;
            }
            match kid {
                PASSWORD_KID => {
                    let password = inputs
                        .password
                        .ok_or(FormatError::RekeyCredentialRequired("password"))?;
                    self.require_unwraps_to_cek(&Credential::Password(password))?;
                    let header: PasswordRecipientHeader =
                        serde_json::from_value(recipient.header.clone()).map_err(|_| {
                            FormatError::Malformed("password recipient header incomplete")
                        })?;
                    let params = KdfParams::from_untrusted(
                        header.argon2_m,
                        header.argon2_t,
                        header.argon2_p,
                    )?;
                    next.set_password(password, params)?;
                }
                SEED_KID => match inputs.new_recovery_entropy {
                    Some(entropy) => next.set_recovery_seed(entropy)?,
                    None => dropped.push(DroppedRecipient::Recovery),
                },
                FIDO2_KID => match inputs.fido2_secret {
                    Some(secret) => {
                        if self
                            .require_unwraps_to_cek(&Credential::Fido2Secret(secret))
                            .is_err()
                        {
                            dropped.push(DroppedRecipient::Fido2);
                            continue;
                        }
                        let header: Fido2RecipientHeader =
                            serde_json::from_value(recipient.header.clone()).map_err(|_| {
                                FormatError::Malformed("fido2 recipient header incomplete")
                            })?;
                        let salt: [u8; 32] = decode_b64(&header.salt)?
                            .try_into()
                            .map_err(|_| FormatError::Malformed("fido2 salt is not 32 bytes"))?;
                        next.set_fido2_recipient(
                            &decode_b64(&header.credential_id)?,
                            &salt,
                            &header.rp_id,
                            secret,
                        )?;
                    }
                    None => dropped.push(DroppedRecipient::Fido2),
                },
                TWO_FACTOR_KID => self.rewrap_two_factor(&mut next, recipient, inputs)?,
                PLATFORM_KID => match inputs.platform_kek {
                    Some(kek) => {
                        if self
                            .require_unwraps_to_cek(&Credential::PlatformKey(kek))
                            .is_err()
                        {
                            dropped.push(DroppedRecipient::Platform);
                            continue;
                        }
                        let header: PlatformRecipientHeader =
                            serde_json::from_value(recipient.header.clone()).map_err(|_| {
                                FormatError::Malformed("platform recipient header incomplete")
                            })?;
                        let keychain_id: [u8; 16] = decode_b64(&header.keychain_id)?
                            .try_into()
                            .map_err(|_| FormatError::Malformed("keychainId is not 16 bytes"))?;
                        next.set_platform_recipient(&keychain_id, kek)?;
                    }
                    None => dropped.push(DroppedRecipient::Platform),
                },
                other => dropped.push(DroppedRecipient::Unknown(other.to_string())),
            }
        }

        if next.recipients.is_empty() {
            return Err(FormatError::LastRecipient);
        }
        *self = next;
        Ok(RekeyReport { dropped })
    }

    fn rewrap_two_factor(
        &self,
        next: &mut Self,
        recipient: &JweRecipient,
        inputs: &RekeyInputs<'_>,
    ) -> Result<(), FormatError> {
        let password = inputs
            .password
            .ok_or(FormatError::RekeyCredentialRequired("password"))?;
        let secret = inputs
            .fido2_secret
            .ok_or(FormatError::RekeyCredentialRequired("fido2"))?;
        self.require_unwraps_to_cek(&Credential::PasswordAndFido2 {
            password,
            fido2_secret: secret,
        })?;
        let header: TwoFactorRecipientHeader = serde_json::from_value(recipient.header.clone())
            .map_err(|_| FormatError::Malformed("two-factor recipient header incomplete"))?;
        let params = KdfParams::from_untrusted(header.argon2_m, header.argon2_t, header.argon2_p)?;
        let salt: [u8; 32] = decode_b64(&header.salt)?
            .try_into()
            .map_err(|_| FormatError::Malformed("fido2 salt is not 32 bytes"))?;
        next.set_two_factor_recipient(
            password,
            params,
            &decode_b64(&header.credential_id)?,
            &salt,
            &header.rp_id,
            secret,
        )
    }

    pub fn check_password(&self, password: &[u8]) -> Result<(), FormatError> {
        self.require_unwraps_to_cek(&Credential::Password(password))
    }

    #[must_use]
    pub fn has_password_recipient(&self) -> bool {
        find_recipient(&self.recipients, PASSWORD_KID).is_some()
    }

    pub fn check_credential(&self, credential: &Credential<'_>) -> Result<(), FormatError> {
        self.require_unwraps_to_cek(credential)
    }

    fn require_unwraps_to_cek(&self, credential: &Credential<'_>) -> Result<(), FormatError> {
        if unwrap_cek(&self.recipients, credential)? == self.cek {
            Ok(())
        } else {
            Err(FormatError::DecryptFailed)
        }
    }

    pub fn rotate_seed(&mut self) -> Result<SeedId, FormatError> {
        let new_id = loop {
            let candidate = random_seed_id()?;
            if !self.seeds.contains_key(&candidate) {
                break candidate;
            }
        };
        let new_seed = Secret::<32>::random()?;
        self.seeds.insert(new_id, new_seed);
        self.latest_seed_id = new_id;
        Ok(new_id)
    }

    pub fn serialize(&self) -> Result<Vec<u8>, FormatError> {
        let mut seeds = HashMap::with_capacity(self.seeds.len());
        for (id, seed) in &self.seeds {
            seeds.insert(b64(id.as_bytes()), b64(seed.expose()));
        }
        let mut payload_map = self.extra_payload_fields.clone();
        payload_map.insert(
            "fileFormat".to_string(),
            Value::String(self.profile.file_format().to_string()),
        );
        payload_map.insert(
            "nameFormat".to_string(),
            Value::String(self.profile.name_format().to_string()),
        );
        payload_map.insert(
            "seeds".to_string(),
            serde_json::to_value(seeds)
                .map_err(|_| FormatError::Malformed("payload cannot be serialized"))?,
        );
        payload_map.insert(
            "initialSeed".to_string(),
            Value::String(b64(self.initial_seed_id.as_bytes())),
        );
        payload_map.insert(
            "latestSeed".to_string(),
            Value::String(b64(self.latest_seed_id.as_bytes())),
        );
        payload_map.insert("kdf".to_string(), Value::String(KDF_NAME.to_string()));
        payload_map.insert("kdfSalt".to_string(), Value::String(b64(&self.kdf_salt)));
        if let Some(public_key) = &self.inbox_public_key {
            payload_map.insert(
                "org.cryptoupload.inbox.publicKey".to_string(),
                Value::String(b64(public_key)),
            );
        }
        if !self.policy.is_default() {
            payload_map.insert(POLICY_FIELD.to_string(), self.policy.to_value()?);
        }
        if self.decoy_entries {
            payload_map.insert(DECOY_ENTRIES_FIELD.to_string(), Value::Bool(true));
        }
        if self.config_seq != 0 {
            payload_map.insert(
                CONFIG_SEQ_FIELD.to_string(),
                Value::Number(self.config_seq.into()),
            );
        }
        let member_names: std::collections::BTreeMap<&String, &String> = self
            .member_names
            .iter()
            .filter(|(kid, _)| find_recipient(&self.recipients, kid).is_some())
            .collect();
        if !member_names.is_empty() {
            payload_map.insert(
                MEMBER_NAMES_FIELD.to_string(),
                serde_json::to_value(&member_names)
                    .map_err(|_| FormatError::Malformed("payload cannot be serialized"))?,
            );
        }

        let payload_bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(&Value::Object(payload_map))
                .map_err(|_| FormatError::Malformed("payload cannot be serialized"))?,
        );
        let sealed = cu_crypto::aead::seal_with_aad(
            &self.cek,
            PROTECTED_HEADER_B64URL.as_bytes(),
            &payload_bytes,
        )?;
        let (iv, ciphertext, tag) = split_sealed(&sealed);

        let doc = JweDoc {
            protected: PROTECTED_HEADER_B64URL.to_string(),
            recipients: self.recipients.clone(),
            iv: b64(&iv),
            ciphertext: b64(&ciphertext),
            tag: b64(&tag),
        };
        serde_json::to_vec(&doc)
            .map_err(|_| FormatError::Malformed("JWE document cannot be serialized"))
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct JweDoc {
    protected: String,
    recipients: Vec<JweRecipient>,
    iv: String,
    ciphertext: String,
    tag: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct JweRecipient {
    header: serde_json::Value,
    encrypted_key: String,
}

#[derive(Deserialize)]
struct ProtectedHeader {
    enc: String,
    cty: String,
    crit: Vec<String>,
    #[serde(rename = "uvf.spec.version")]
    spec_version: u8,
    #[serde(default)]
    zip: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
struct PasswordRecipientHeader {
    alg: String,
    kid: String,
    #[serde(rename = "org.cryptoupload.argon2.salt")]
    argon2_salt: String,
    #[serde(rename = "org.cryptoupload.argon2.m")]
    argon2_m: u32,
    #[serde(rename = "org.cryptoupload.argon2.t")]
    argon2_t: u32,
    #[serde(rename = "org.cryptoupload.argon2.p")]
    argon2_p: u32,
}

#[derive(Serialize, Deserialize)]
struct SeedRecipientHeader {
    alg: String,
    kid: String,
    #[serde(rename = "org.cryptoupload.seed.salt")]
    seed_salt: String,
}

#[derive(Serialize, Deserialize)]
struct Fido2RecipientHeader {
    alg: String,
    kid: String,
    #[serde(rename = "org.cryptoupload.fido2.credentialId")]
    credential_id: String,
    #[serde(rename = "org.cryptoupload.fido2.salt")]
    salt: String,
    #[serde(rename = "org.cryptoupload.fido2.rpId")]
    rp_id: String,
}

#[derive(Serialize, Deserialize)]
struct TwoFactorRecipientHeader {
    alg: String,
    kid: String,
    #[serde(rename = "org.cryptoupload.argon2.salt")]
    argon2_salt: String,
    #[serde(rename = "org.cryptoupload.argon2.m")]
    argon2_m: u32,
    #[serde(rename = "org.cryptoupload.argon2.t")]
    argon2_t: u32,
    #[serde(rename = "org.cryptoupload.argon2.p")]
    argon2_p: u32,
    #[serde(rename = "org.cryptoupload.fido2.credentialId")]
    credential_id: String,
    #[serde(rename = "org.cryptoupload.fido2.salt")]
    salt: String,
    #[serde(rename = "org.cryptoupload.fido2.rpId")]
    rp_id: String,
}

fn two_factor_kek(
    password: &[u8],
    argon2_salt: &[u8],
    argon2_params: KdfParams,
    fido2_salt: &[u8],
    hmac_secret: &Secret<32>,
) -> Result<Secret<32>, FormatError> {
    let from_password = cu_crypto::kdf::derive_kek(password, argon2_salt, argon2_params)?;
    let mut ikm = zeroize::Zeroizing::new(Vec::with_capacity(64));
    ikm.extend_from_slice(from_password.expose());
    ikm.extend_from_slice(hmac_secret.expose());
    Ok(cu_crypto::hkdf::derive_secret(
        &ikm,
        Some(fido2_salt),
        TWO_FACTOR_KEK_INFO,
    )?)
}

#[derive(Serialize, Deserialize)]
struct PlatformRecipientHeader {
    alg: String,
    kid: String,
    #[serde(rename = "org.cryptoupload.platform.keychainId")]
    keychain_id: String,
}

#[derive(Serialize, Deserialize)]
struct MemberRecipientHeader {
    alg: String,
    kid: String,
    epk: String,
    apu: String,
    apv: String,
    #[serde(rename = "org.cryptoupload.member.publicKey")]
    public_key: String,
    #[serde(rename = "org.cryptoupload.member.name")]
    name: String,
}

#[must_use]
pub fn member_kid(public_key: &[u8; 32]) -> String {
    format!("{MEMBER_KID_PREFIX}{}", b64(public_key))
}

#[derive(Serialize, Deserialize)]
struct PayloadJson {
    #[serde(rename = "fileFormat")]
    file_format: String,
    #[serde(rename = "nameFormat")]
    name_format: String,
    seeds: HashMap<String, String>,
    #[serde(rename = "initialSeed")]
    initial_seed: String,
    #[serde(rename = "latestSeed")]
    latest_seed: String,
    kdf: String,
    #[serde(rename = "kdfSalt")]
    kdf_salt: String,
    #[serde(
        rename = "org.cryptoupload.inbox.publicKey",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    inbox_public_key: Option<String>,
    #[serde(
        rename = "org.cryptoupload.policy",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    policy: Option<Value>,
    #[serde(
        rename = "org.cryptoupload.memberNames",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    member_names: Option<std::collections::BTreeMap<String, String>>,
    #[serde(
        rename = "org.cryptoupload.decoyEntries",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    decoy_entries: bool,
    #[serde(
        rename = "org.cryptoupload.configSeq",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    config_seq: Option<u64>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

pub fn create(password: &[u8], argon2_params: KdfParams) -> Result<VaultConfig, FormatError> {
    create_with_profile(password, argon2_params, FormatProfile::Standard)
}

pub fn create_with_profile(
    password: &[u8],
    argon2_params: KdfParams,
    profile: FormatProfile,
) -> Result<VaultConfig, FormatError> {
    let initial_seed = Secret::<32>::random()?;
    let initial_seed_id = random_seed_id()?;
    let kdf_salt: [u8; 32] = random_array()?;
    let cek = Secret::<32>::random()?;

    let mut seeds = HashMap::new();
    seeds.insert(initial_seed_id, initial_seed);

    let mut config = VaultConfig {
        seeds,
        initial_seed_id,
        latest_seed_id: initial_seed_id,
        kdf_salt,
        cek,
        recipients: Vec::new(),
        inbox_public_key: None,
        policy: VaultPolicy::default(),
        profile,
        member_names: std::collections::BTreeMap::new(),
        decoy_entries: false,
        config_seq: 1,
        extra_payload_fields: Map::new(),
    };
    config.set_password(password, argon2_params)?;
    Ok(config)
}

pub fn open(bytes: &[u8], credential: &Credential<'_>) -> Result<VaultConfig, FormatError> {
    let doc: JweDoc = serde_json::from_slice(bytes)
        .map_err(|_| FormatError::Malformed("vault.uvf is not a valid JWE JSON"))?;

    validate_protected_header(&doc.protected)?;
    reject_duplicate_kids(&doc.recipients)?;

    let cek = unwrap_cek(&doc.recipients, credential)?;

    let iv = decode_b64(&doc.iv)?;
    let ciphertext = decode_b64(&doc.ciphertext)?;
    let tag = decode_b64(&doc.tag)?;
    let mut sealed = Vec::with_capacity(iv.len() + ciphertext.len() + tag.len());
    sealed.extend_from_slice(&iv);
    sealed.extend_from_slice(&ciphertext);
    sealed.extend_from_slice(&tag);
    let payload_bytes = zeroize::Zeroizing::new(cu_crypto::aead::open_with_aad(
        &cek,
        doc.protected.as_bytes(),
        &sealed,
    )?);

    let mut payload: PayloadJson = serde_json::from_slice(&payload_bytes)
        .map_err(|_| FormatError::Malformed("vault.uvf payload is not valid JSON"))?;
    let profile = if payload.file_format == FILE_FORMAT {
        FormatProfile::Standard
    } else if payload.file_format == PRIVATE_FILE_FORMAT {
        FormatProfile::Private
    } else {
        return Err(FormatError::UnknownAlgorithm(
            "fileFormat",
            payload.file_format,
        ));
    };
    if payload.name_format != profile.name_format() {
        return Err(FormatError::UnknownAlgorithm(
            "nameFormat",
            payload.name_format,
        ));
    }
    if payload.kdf != KDF_NAME {
        return Err(FormatError::UnknownAlgorithm("kdf", payload.kdf));
    }

    let kdf_salt: [u8; 32] = decode_b64(&payload.kdf_salt)?
        .try_into()
        .map_err(|_| FormatError::Malformed("kdfSalt is not 32 bytes"))?;

    let mut seeds = HashMap::with_capacity(payload.seeds.len());
    for (id_b64, seed_b64) in &payload.seeds {
        let id: [u8; 4] = decode_b64(id_b64)?
            .try_into()
            .map_err(|_| FormatError::Malformed("seed ID is not 4 bytes"))?;
        let seed: [u8; 32] = decode_b64(seed_b64)?
            .try_into()
            .map_err(|_| FormatError::Malformed("seed is not 32 bytes"))?;
        seeds.insert(SeedId(id), Secret::new(seed));
    }
    for seed_b64 in payload.seeds.values_mut() {
        zeroize::Zeroize::zeroize(seed_b64);
    }
    let initial_seed_id = seed_id_field(&payload.initial_seed, &seeds, "initialSeed")?;
    let latest_seed_id = seed_id_field(&payload.latest_seed, &seeds, "latestSeed")?;

    let inbox_public_key = match &payload.inbox_public_key {
        Some(b64_str) => {
            let bytes: [u8; 32] = decode_b64(b64_str)?.try_into().map_err(|_| {
                FormatError::Malformed("org.cryptoupload.inbox.publicKey is not 32 bytes")
            })?;
            Some(bytes)
        }
        None => None,
    };

    let policy = match &payload.policy {
        Some(value) => VaultPolicy::from_value(value)?,
        None => VaultPolicy::default(),
    };

    Ok(VaultConfig {
        seeds,
        initial_seed_id,
        latest_seed_id,
        kdf_salt,
        cek,
        recipients: doc.recipients,
        inbox_public_key,
        policy,
        profile,
        member_names: payload.member_names.take().unwrap_or_default(),
        decoy_entries: payload.decoy_entries && profile == FormatProfile::Private,
        config_seq: payload.config_seq.unwrap_or(0),
        extra_payload_fields: payload.extra,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipientInfo {
    Password,
    RecoverySeed,
    Fido2 {
        credential_id: Vec<u8>,
        salt: [u8; 32],
        rp_id: String,
    },
    TwoFactor {
        credential_id: Vec<u8>,
        salt: [u8; 32],
        rp_id: String,
    },
    Platform {
        keychain_id: [u8; 16],
    },
    Member {
        name: String,
        public_key: [u8; 32],
    },
    Unknown(String),
}

pub fn peek_recipients(bytes: &[u8]) -> Result<Vec<RecipientInfo>, FormatError> {
    let doc: JweDoc = serde_json::from_slice(bytes)
        .map_err(|_| FormatError::Malformed("vault.uvf is not a valid JWE JSON"))?;
    validate_protected_header(&doc.protected)?;
    reject_duplicate_kids(&doc.recipients)?;
    doc.recipients.iter().map(recipient_info_of).collect()
}

fn recipient_info_of(r: &JweRecipient) -> Result<RecipientInfo, FormatError> {
    let kid = r.header.get("kid").and_then(Value::as_str).unwrap_or("");
    if kid.starts_with(MEMBER_KID_PREFIX) {
        let header: MemberRecipientHeader = serde_json::from_value(r.header.clone())
            .map_err(|_| FormatError::Malformed("member recipient header incomplete"))?;
        let public_key: [u8; 32] = decode_b64(&header.public_key)?
            .try_into()
            .map_err(|_| FormatError::Malformed("member public key is not 32 bytes"))?;
        return Ok(RecipientInfo::Member {
            name: header.name,
            public_key,
        });
    }
    match kid {
        PASSWORD_KID => Ok(RecipientInfo::Password),
        SEED_KID => Ok(RecipientInfo::RecoverySeed),
        FIDO2_KID => {
            let header: Fido2RecipientHeader = serde_json::from_value(r.header.clone())
                .map_err(|_| FormatError::Malformed("fido2 recipient header incomplete"))?;
            let credential_id = decode_b64(&header.credential_id)?;
            let salt: [u8; 32] = decode_b64(&header.salt)?
                .try_into()
                .map_err(|_| FormatError::Malformed("fido2 salt is not 32 bytes"))?;
            Ok(RecipientInfo::Fido2 {
                credential_id,
                salt,
                rp_id: header.rp_id,
            })
        }
        TWO_FACTOR_KID => {
            let header: TwoFactorRecipientHeader = serde_json::from_value(r.header.clone())
                .map_err(|_| FormatError::Malformed("two-factor recipient header incomplete"))?;
            let salt: [u8; 32] = decode_b64(&header.salt)?
                .try_into()
                .map_err(|_| FormatError::Malformed("fido2 salt is not 32 bytes"))?;
            Ok(RecipientInfo::TwoFactor {
                credential_id: decode_b64(&header.credential_id)?,
                salt,
                rp_id: header.rp_id,
            })
        }
        PLATFORM_KID => {
            let header: PlatformRecipientHeader = serde_json::from_value(r.header.clone())
                .map_err(|_| FormatError::Malformed("platform recipient header incomplete"))?;
            let keychain_id: [u8; 16] = decode_b64(&header.keychain_id)?
                .try_into()
                .map_err(|_| FormatError::Malformed("keychainId is not 16 bytes"))?;
            Ok(RecipientInfo::Platform { keychain_id })
        }
        other => Ok(RecipientInfo::Unknown(other.to_string())),
    }
}

fn password_recipient(recipients: &[JweRecipient]) -> Result<&JweRecipient, FormatError> {
    match find_recipient(recipients, PASSWORD_KID) {
        Some(recipient) => Ok(recipient),
        None if find_recipient(recipients, TWO_FACTOR_KID).is_some() => {
            Err(FormatError::TwoFactorRequired)
        }
        None => Err(FormatError::Malformed("no password recipient in vault.uvf")),
    }
}

fn unwrap_two_factor(
    recipients: &[JweRecipient],
    password: &[u8],
    fido2_secret: &Secret<32>,
) -> Result<Secret<32>, FormatError> {
    let recipient =
        find_recipient(recipients, TWO_FACTOR_KID).ok_or(FormatError::NoSuchRecipient)?;
    let header: TwoFactorRecipientHeader = serde_json::from_value(recipient.header.clone())
        .map_err(|_| FormatError::Malformed("two-factor recipient header incomplete"))?;
    if header.alg != TWO_FACTOR_ALG {
        return Err(FormatError::Malformed(
            "unexpected two-factor recipient alg",
        ));
    }
    let argon2_params =
        KdfParams::from_untrusted(header.argon2_m, header.argon2_t, header.argon2_p)?;
    let kek = two_factor_kek(
        password,
        &decode_b64(&header.argon2_salt)?,
        argon2_params,
        &decode_b64(&header.salt)?,
        fido2_secret,
    )?;
    unwrap_to_cek(&kek, &decode_b64(&recipient.encrypted_key)?)
}

fn unwrap_cek(
    recipients: &[JweRecipient],
    credential: &Credential<'_>,
) -> Result<Secret<32>, FormatError> {
    match credential {
        Credential::PasswordAndFido2 {
            password,
            fido2_secret,
        } => unwrap_two_factor(recipients, password, fido2_secret),
        Credential::Password(password) => {
            let recipient = password_recipient(recipients)?;
            let header: PasswordRecipientHeader = serde_json::from_value(recipient.header.clone())
                .map_err(|_| FormatError::Malformed("password recipient header incomplete"))?;
            if header.alg != PASSWORD_ALG {
                return Err(FormatError::Malformed("unexpected password recipient alg"));
            }
            let argon2_salt = decode_b64(&header.argon2_salt)?;
            let argon2_params =
                KdfParams::from_untrusted(header.argon2_m, header.argon2_t, header.argon2_p)?;
            let kek_pw = cu_crypto::kdf::derive_kek(password, &argon2_salt, argon2_params)?;
            let encrypted_key = decode_b64(&recipient.encrypted_key)?;
            unwrap_to_cek(&kek_pw, &encrypted_key)
        }
        Credential::RecoverySeed(entropy) => {
            let recipient =
                find_recipient(recipients, SEED_KID).ok_or(FormatError::NoRecoveryRecipient)?;
            let header: SeedRecipientHeader = serde_json::from_value(recipient.header.clone())
                .map_err(|_| FormatError::Malformed("seed recipient header incomplete"))?;
            if header.alg != SEED_ALG {
                return Err(FormatError::Malformed("unexpected seed recipient alg"));
            }
            let seed_salt = decode_b64(&header.seed_salt)?;
            let kek_seed =
                cu_crypto::hkdf::derive_secret(entropy.expose(), Some(&seed_salt), SEED_KEK_INFO)?;
            let encrypted_key = decode_b64(&recipient.encrypted_key)?;
            unwrap_to_cek(&kek_seed, &encrypted_key)
        }
        Credential::Fido2Secret(hmac_secret) => {
            let recipient =
                find_recipient(recipients, FIDO2_KID).ok_or(FormatError::NoFido2Recipient)?;
            let header: Fido2RecipientHeader = serde_json::from_value(recipient.header.clone())
                .map_err(|_| FormatError::Malformed("fido2 recipient header incomplete"))?;
            if header.alg != FIDO2_ALG {
                return Err(FormatError::Malformed("unexpected fido2 recipient alg"));
            }
            let salt = decode_b64(&header.salt)?;
            let kek_fido2 =
                cu_crypto::hkdf::derive_secret(hmac_secret.expose(), Some(&salt), FIDO2_KEK_INFO)?;
            let encrypted_key = decode_b64(&recipient.encrypted_key)?;
            unwrap_to_cek(&kek_fido2, &encrypted_key)
        }
        Credential::PlatformKey(kek) => {
            let recipient =
                find_recipient(recipients, PLATFORM_KID).ok_or(FormatError::NoPlatformRecipient)?;
            let header: PlatformRecipientHeader = serde_json::from_value(recipient.header.clone())
                .map_err(|_| FormatError::Malformed("platform recipient header incomplete"))?;
            if header.alg != PLATFORM_ALG {
                return Err(FormatError::Malformed("unexpected platform recipient alg"));
            }
            let encrypted_key = decode_b64(&recipient.encrypted_key)?;
            unwrap_to_cek(kek, &encrypted_key)
        }
        Credential::MemberKey(keypair) => {
            let kid = member_kid(&keypair.public_bytes());
            let recipient =
                find_recipient(recipients, &kid).ok_or(FormatError::NoMemberRecipient)?;
            let header: MemberRecipientHeader = serde_json::from_value(recipient.header.clone())
                .map_err(|_| FormatError::Malformed("member recipient header incomplete"))?;
            if header.alg != MEMBER_ALG {
                return Err(FormatError::Malformed("unexpected member recipient alg"));
            }
            let epk: [u8; 32] = decode_b64(&header.epk)?
                .try_into()
                .map_err(|_| FormatError::Malformed("member recipient epk is not 32 bytes"))?;
            let apu = decode_b64(&header.apu)?;
            let apv = decode_b64(&header.apv)?;
            let z = keypair
                .diffie_hellman(&epk)
                .map_err(|_| FormatError::DecryptFailed)?;
            let kek_bytes = cu_crypto::concat_kdf::derive(
                z.expose(),
                MEMBER_KEK_BITS,
                MEMBER_ALG.as_bytes(),
                &apu,
                &apv,
            )?;
            let kek_array: [u8; 32] = kek_bytes
                .as_slice()
                .try_into()
                .map_err(|_| FormatError::DecryptFailed)?;
            let kek = Secret::new(kek_array);
            let encrypted_key = decode_b64(&recipient.encrypted_key)?;
            unwrap_to_cek(&kek, &encrypted_key)
        }
    }
}

fn unwrap_to_cek(kek: &Secret<32>, encrypted_key: &[u8]) -> Result<Secret<32>, FormatError> {
    let cek_bytes = cu_crypto::keywrap::unwrap(kek, encrypted_key)?;
    let cek_bytes: [u8; 32] = cek_bytes
        .as_slice()
        .try_into()
        .map_err(|_| FormatError::DecryptFailed)?;
    Ok(Secret::new(cek_bytes))
}

fn find_recipient<'a>(recipients: &'a [JweRecipient], kid: &str) -> Option<&'a JweRecipient> {
    recipients
        .iter()
        .find(|r| r.header.get("kid").and_then(Value::as_str) == Some(kid))
}

fn replace_recipient(recipients: &mut Vec<JweRecipient>, kid: &str, new: JweRecipient) {
    if let Some(existing) = recipients
        .iter_mut()
        .find(|r| r.header.get("kid").and_then(Value::as_str) == Some(kid))
    {
        *existing = new;
    } else {
        recipients.push(new);
    }
}

fn reject_duplicate_kids(recipients: &[JweRecipient]) -> Result<(), FormatError> {
    let mut seen = std::collections::HashSet::new();
    for r in recipients {
        if let Some(kid) = r.header.get("kid").and_then(Value::as_str)
            && !seen.insert(kid)
        {
            return Err(FormatError::Malformed(
                "duplicate kid among the recipients in vault.uvf",
            ));
        }
    }
    Ok(())
}

fn validate_protected_header(protected_b64url: &str) -> Result<(), FormatError> {
    let decoded = decode_b64(protected_b64url)?;
    let header: ProtectedHeader = serde_json::from_slice(&decoded)
        .map_err(|_| FormatError::Malformed("JWE protected header is not valid JSON"))?;
    if header.zip.is_some() {
        return Err(FormatError::Malformed(
            "zip not allowed in the JWE protected header",
        ));
    }
    if header.enc != "A256GCM" {
        return Err(FormatError::Malformed(
            "unexpected JWE protected header enc",
        ));
    }
    if header.cty != "json" {
        return Err(FormatError::Malformed(
            "unexpected JWE protected header cty",
        ));
    }
    if header.crit != ["uvf.spec.version"] {
        return Err(FormatError::Malformed(
            "unexpected JWE protected header crit",
        ));
    }
    if header.spec_version != SPEC_VERSION {
        return Err(FormatError::UnsupportedVersion(header.spec_version));
    }
    Ok(())
}

fn seed_id_field(
    b64url: &str,
    seeds: &HashMap<SeedId, Secret<32>>,
    field_name: &'static str,
) -> Result<SeedId, FormatError> {
    let id: [u8; 4] = decode_b64(b64url)?
        .try_into()
        .map_err(|_| FormatError::Malformed(field_name))?;
    let id = SeedId(id);
    if !seeds.contains_key(&id) {
        return Err(FormatError::Malformed(field_name));
    }
    Ok(id)
}

fn random_seed_id() -> Result<SeedId, FormatError> {
    Ok(SeedId(random_array()?))
}

fn random_array<const N: usize>() -> Result<[u8; N], FormatError> {
    let mut buf = [0u8; N];
    cu_crypto::random::fill_random(&mut buf)?;
    Ok(buf)
}

fn b64(bytes: &[u8]) -> String {
    BASE64URL_NOPAD.encode(bytes)
}

fn decode_b64(s: &str) -> Result<Vec<u8>, FormatError> {
    BASE64URL_NOPAD
        .decode(s.as_bytes())
        .map_err(|_| FormatError::Malformed("field is not valid base64url"))
}

fn split_sealed(sealed: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    const NONCE_LEN: usize = cu_crypto::aead::NONCE_LEN;
    const TAG_LEN: usize = cu_crypto::aead::TAG_LEN;
    let (iv, rest) = sealed.split_at(NONCE_LEN);
    let (ciphertext, tag) = rest.split_at(rest.len() - TAG_LEN);
    (iv.to_vec(), ciphertext.to_vec(), tag.to_vec())
}
