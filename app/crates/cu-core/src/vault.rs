// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg};
use std::path::Path;
use std::sync::Arc;

use cu_auth_fido2::Fido2Authenticator;
use cu_backend::{BackendError, ObjPath, StorageBackend};
use cu_crypto::Secret;
use cu_crypto::kdf::KdfParams;
use cu_crypto::seed::RecoveryPhrase;
use cu_crypto::x25519::StaticKeypair;
use cu_format::config::{self, Credential, FormatProfile};
use cu_format::error::FormatError;
use cu_format::policy::VaultPolicy;
use cu_vfs::VaultFs;

use crate::error::CoreError;
use crate::guard;
use crate::unlock_methods::{self, UnlockMethod};

pub use cu_vfs::{DetailedEntry, Entry, NodeKind};

const EXPORT_BUFFER: usize = 1024 * 1024;

const README_TEXT: &str = "This is an encrypted CryptoUpload vault.\n\
\n\
Do not edit, rename or move the files inside this folder by hand: they are encrypted and\n\
their structure is part of the format. Use the CryptoUpload app to add, list or extract files.\n\
\n\
CryptoUpload has not yet had an independent security audit. Do not rely on this vault as the\n\
only protection for information whose disclosure would cause you serious harm.\n";

#[derive(Debug, Clone)]
pub enum ProgressEvent {
    File { vault_path: String },
    Bytes { count: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberInfo {
    pub name: String,
    pub public_key: [u8; 32],
}

pub struct Fido2Auth<'a> {
    pub authenticator: &'a dyn Fido2Authenticator,
    pub pin: Option<&'a str>,
}

pub struct RemoveMemberAuth<'a> {
    pub password: &'a [u8],
    pub fido2: Option<Fido2Auth<'a>>,
}

pub struct RemoveMemberOutcome {
    pub new_recovery_phrase: Option<RecoveryPhrase>,
    pub dropped: Vec<config::DroppedRecipient>,
}

pub struct Vault {
    pub(crate) fs: Arc<VaultFs>,
    memory_locks: Option<crate::memlock::MemoryLocks>,
    config: Box<config::VaultConfig>,
    config_digest: [u8; 32],
    previous_config_digest: Option<[u8; 32]>,
    pub(crate) registry: Option<Arc<cu_sync::registry::Registrar>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaultCreation {
    pub profile: FormatProfile,
    pub decoy_files: bool,
    pub readme: Option<String>,
}

impl Vault {
    pub async fn create_local(root: &Path, password: &[u8]) -> Result<Self, CoreError> {
        Self::create_local_with_bandwidth(root, password, None).await
    }

    pub async fn create_local_with_bandwidth(
        root: &Path,
        password: &[u8],
        bandwidth_limit_kib: Option<u32>,
    ) -> Result<Self, CoreError> {
        Self::create_local_in_profile(
            root,
            password,
            bandwidth_limit_kib,
            VaultCreation::default(),
        )
        .await
    }

    pub async fn create_local_in_profile(
        root: &Path,
        password: &[u8],
        bandwidth_limit_kib: Option<u32>,
        creation: VaultCreation,
    ) -> Result<Self, CoreError> {
        check_new_password(password)?;
        guard::check_not_inside_third_party_sync(root)?;
        tokio::fs::create_dir_all(root).await?;
        let mut existing = tokio::fs::read_dir(root).await?;
        if existing.next_entry().await?.is_some() {
            return Err(CoreError::AlreadyExists);
        }
        let backend: Box<dyn StorageBackend> = Box::new(cu_backend_fs::FsBackend::new(root));
        let backend = crate::remote::wrap_bandwidth(backend, bandwidth_limit_kib);
        Self::create_with_backend(backend, password, creation).await
    }

    pub async fn create_remote(
        cfg: &crate::remote::RemoteConfig,
        remote_state_db: &Path,
        password: &[u8],
        bandwidth_limit_kib: Option<u32>,
    ) -> Result<Self, CoreError> {
        Self::create_remote_with_refresh_sink(
            cfg,
            remote_state_db,
            password,
            bandwidth_limit_kib,
            None,
        )
        .await
    }

    pub async fn create_remote_with_refresh_sink(
        cfg: &crate::remote::RemoteConfig,
        remote_state_db: &Path,
        password: &[u8],
        bandwidth_limit_kib: Option<u32>,
        refresh_sink: Option<crate::remote::RefreshTokenSink>,
    ) -> Result<Self, CoreError> {
        Self::create_remote_in_profile(
            cfg,
            remote_state_db,
            password,
            bandwidth_limit_kib,
            refresh_sink,
            VaultCreation::default(),
        )
        .await
    }

    pub async fn create_remote_in_profile(
        cfg: &crate::remote::RemoteConfig,
        remote_state_db: &Path,
        password: &[u8],
        bandwidth_limit_kib: Option<u32>,
        refresh_sink: Option<crate::remote::RefreshTokenSink>,
        creation: VaultCreation,
    ) -> Result<Self, CoreError> {
        check_new_password(password)?;
        let backend = crate::remote::build_backend_with_refresh_sink(
            cfg,
            remote_state_db,
            bandwidth_limit_kib,
            refresh_sink,
        )
        .await?;
        Self::create_with_backend(backend, password, creation).await
    }

    pub(crate) async fn create_with_backend(
        backend: Box<dyn StorageBackend>,
        password: &[u8],
        creation: VaultCreation,
    ) -> Result<Self, CoreError> {
        check_new_password(password)?;
        let profile = creation.profile;
        let config = off_worker(|| {
            let mut config =
                config::create_with_profile(password, KdfParams::sensitive(), profile)?;
            config.set_decoy_entries(creation.decoy_files)?;
            Ok::<_, FormatError>(config)
        })?;
        let vault_uvf_bytes = config.serialize()?;
        backend
            .put(&ObjPath::root().join("vault.uvf")?, &vault_uvf_bytes)
            .await?;
        if profile == FormatProfile::Standard {
            backend
                .put(
                    &ObjPath::root().join("README.txt")?,
                    creation.readme.as_deref().unwrap_or(README_TEXT).as_bytes(),
                )
                .await?;
        }

        let fs = VaultFs::new(backend);
        fs.init_root(&config).await?;
        Self {
            fs: Arc::new(fs),
            memory_locks: None,
            config: Box::new(config),
            config_digest: digest_of(&vault_uvf_bytes),
            previous_config_digest: None,
            registry: None,
        }
        .with_global_registry()
    }

    pub async fn open_local(root: &Path, credential: &Credential<'_>) -> Result<Self, CoreError> {
        Self::open_local_with_bandwidth(root, credential, None).await
    }

    pub async fn open_local_with_bandwidth(
        root: &Path,
        credential: &Credential<'_>,
        bandwidth_limit_kib: Option<u32>,
    ) -> Result<Self, CoreError> {
        let backend: Box<dyn StorageBackend> = Box::new(cu_backend_fs::FsBackend::new(root));
        let backend = crate::remote::wrap_bandwidth(backend, bandwidth_limit_kib);
        Self::open_with_backend(backend, credential).await
    }

    pub async fn open_remote(
        cfg: &crate::remote::RemoteConfig,
        remote_state_db: &Path,
        credential: &Credential<'_>,
        bandwidth_limit_kib: Option<u32>,
    ) -> Result<Self, CoreError> {
        Self::open_remote_with_refresh_sink(
            cfg,
            remote_state_db,
            credential,
            bandwidth_limit_kib,
            None,
        )
        .await
    }

    pub async fn open_remote_with_refresh_sink(
        cfg: &crate::remote::RemoteConfig,
        remote_state_db: &Path,
        credential: &Credential<'_>,
        bandwidth_limit_kib: Option<u32>,
        refresh_sink: Option<crate::remote::RefreshTokenSink>,
    ) -> Result<Self, CoreError> {
        let backend = crate::remote::build_backend_with_refresh_sink(
            cfg,
            remote_state_db,
            bandwidth_limit_kib,
            refresh_sink,
        )
        .await?;
        Self::open_with_backend(backend, credential).await
    }

    pub async fn open_local_two_factor(
        root: &Path,
        password: &[u8],
        second: &Fido2Auth<'_>,
        bandwidth_limit_kib: Option<u32>,
    ) -> Result<Self, CoreError> {
        let backend: Box<dyn StorageBackend> = Box::new(cu_backend_fs::FsBackend::new(root));
        let backend = crate::remote::wrap_bandwidth(backend, bandwidth_limit_kib);
        Self::open_with_backend_two_factor(backend, password, second).await
    }

    pub async fn open_remote_two_factor(
        cfg: &crate::remote::RemoteConfig,
        remote_state_db: &Path,
        password: &[u8],
        second: &Fido2Auth<'_>,
        bandwidth_limit_kib: Option<u32>,
        refresh_sink: Option<crate::remote::RefreshTokenSink>,
    ) -> Result<Self, CoreError> {
        let backend = crate::remote::build_backend_with_refresh_sink(
            cfg,
            remote_state_db,
            bandwidth_limit_kib,
            refresh_sink,
        )
        .await?;
        Self::open_with_backend_two_factor(backend, password, second).await
    }

    async fn open_with_backend_two_factor(
        backend: Box<dyn StorageBackend>,
        password: &[u8],
        second: &Fido2Auth<'_>,
    ) -> Result<Self, CoreError> {
        let bytes = fetch_vault_uvf(backend.as_ref()).await?;
        let Some((credential_id, salt, rp_id)) =
            two_factor_params(&config::peek_recipients(&bytes)?)
        else {
            return Self::open_with_backend(backend, &Credential::Password(password)).await;
        };
        let secret = Secret::new(
            second
                .authenticator
                .authenticate(&rp_id, &credential_id, &salt, second.pin)
                .map_err(|e| CoreError::Fido2(e.msg()))?,
        );
        let config = off_worker(|| {
            config::open(
                &bytes,
                &Credential::PasswordAndFido2 {
                    password,
                    fido2_secret: &secret,
                },
            )
        })?;
        Self {
            fs: Arc::new(VaultFs::new(backend)),
            memory_locks: None,
            config: Box::new(config),
            config_digest: digest_of(&bytes),
            previous_config_digest: None,
            registry: None,
        }
        .with_global_registry()
    }

    pub(crate) async fn open_with_backend(
        backend: Box<dyn StorageBackend>,
        credential: &Credential<'_>,
    ) -> Result<Self, CoreError> {
        let bytes = match backend.get(&ObjPath::root().join("vault.uvf")?).await {
            Ok(bytes) => bytes,
            Err(BackendError::NotFound) => return Err(CoreError::NotAVault),
            Err(e) => return Err(e.into()),
        };
        let config = off_worker(|| config::open(&bytes, credential))?;
        Self {
            fs: Arc::new(VaultFs::new(backend)),
            memory_locks: None,
            config: Box::new(config),
            config_digest: digest_of(&bytes),
            previous_config_digest: None,
            registry: None,
        }
        .with_global_registry()
    }

    pub async fn peek_unlock_methods(root: &Path) -> Result<Vec<UnlockMethod>, CoreError> {
        let backend: Box<dyn StorageBackend> = Box::new(cu_backend_fs::FsBackend::new(root));
        let bytes = fetch_vault_uvf(backend.as_ref()).await?;
        Ok(config::peek_recipients(&bytes)?
            .into_iter()
            .map(UnlockMethod::from)
            .collect())
    }

    pub async fn open_local_with_platform_key(root: &Path) -> Result<Self, CoreError> {
        let backend: Box<dyn StorageBackend> = Box::new(cu_backend_fs::FsBackend::new(root));
        let bytes = fetch_vault_uvf(backend.as_ref()).await?;
        let keychain_id =
            config::peek_recipients(&bytes)?
                .into_iter()
                .find_map(|info| match info {
                    config::RecipientInfo::Platform { keychain_id } => Some(keychain_id),
                    _ => None,
                });
        let Some(keychain_id) = keychain_id else {
            return Err(FormatError::NoPlatformRecipient.into());
        };
        let kek =
            unlock_methods::load_kek(&keychain_id)?.ok_or(CoreError::PlatformKeyUnavailable)?;
        let config = config::open(&bytes, &Credential::PlatformKey(&kek))?;
        if config.policy().effective().no_unlock_shortcuts {
            return Err(CoreError::PolicyForbids(Msg::new(
                "POLICY_NO_UNLOCK_SHORTCUTS",
            )));
        }
        Self {
            fs: Arc::new(VaultFs::new(backend)),
            memory_locks: None,
            config: Box::new(config),
            config_digest: digest_of(&bytes),
            previous_config_digest: None,
            registry: None,
        }
        .with_global_registry()
    }

    pub async fn forget_platform_unlock_key(root: &Path) -> Result<bool, CoreError> {
        let backend = cu_backend_fs::FsBackend::new(root);
        let bytes = fetch_vault_uvf(&backend).await?;
        let keychain_id =
            config::peek_recipients(&bytes)?
                .into_iter()
                .find_map(|info| match info {
                    config::RecipientInfo::Platform { keychain_id } => Some(keychain_id),
                    _ => None,
                });
        match keychain_id {
            Some(keychain_id) => {
                unlock_methods::forget_kek(&keychain_id)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub async fn open_local_with_fido2(
        root: &Path,
        authenticator: &dyn Fido2Authenticator,
        pin: Option<&str>,
    ) -> Result<Self, CoreError> {
        let backend: Box<dyn StorageBackend> = Box::new(cu_backend_fs::FsBackend::new(root));
        let bytes = fetch_vault_uvf(backend.as_ref()).await?;
        let fido2_params =
            config::peek_recipients(&bytes)?
                .into_iter()
                .find_map(|info| match info {
                    config::RecipientInfo::Fido2 {
                        credential_id,
                        salt,
                        rp_id,
                    } => Some((credential_id, salt, rp_id)),
                    _ => None,
                });
        let Some((credential_id, salt, rp_id)) = fido2_params else {
            return Err(FormatError::NoFido2Recipient.into());
        };
        let secret = authenticator
            .authenticate(&rp_id, &credential_id, &salt, pin)
            .map_err(|e| CoreError::Fido2(e.msg()))?;
        let config = config::open(&bytes, &Credential::Fido2Secret(&Secret::new(secret)))?;
        Self {
            fs: Arc::new(VaultFs::new(backend)),
            memory_locks: None,
            config: Box::new(config),
            config_digest: digest_of(&bytes),
            previous_config_digest: None,
            registry: None,
        }
        .with_global_registry()
    }

    pub async fn open_local_with_member_key(
        root: &Path,
        keypair: &StaticKeypair,
    ) -> Result<Self, CoreError> {
        let backend: Box<dyn StorageBackend> = Box::new(cu_backend_fs::FsBackend::new(root));
        let bytes = fetch_vault_uvf(backend.as_ref()).await?;
        let config = config::open(&bytes, &Credential::MemberKey(keypair))?;
        if let Some(data_dir) = crate::registry::global_data_dir() {
            let vault_id = cu_format::keys::root_dir_id(config.initial_seed(), config.kdf_salt())?;
            crate::vault_pin::check_or_remember(data_dir, root, &vault_id)?;
        }
        Self {
            fs: Arc::new(VaultFs::new(backend)),
            memory_locks: None,
            config: Box::new(config),
            config_digest: digest_of(&bytes),
            previous_config_digest: None,
            registry: None,
        }
        .with_global_registry()
    }

    #[must_use]
    pub fn file_format(&self) -> &'static str {
        self.config.profile().file_format()
    }

    #[must_use]
    pub fn name_format(&self) -> &'static str {
        self.config.profile().name_format()
    }

    #[must_use]
    pub fn kdf_name(&self) -> &'static str {
        config::KDF_NAME
    }

    #[must_use]
    pub fn seed_count(&self) -> usize {
        self.config.seed_count()
    }

    #[must_use]
    pub(crate) fn fs(&self) -> &VaultFs {
        &self.fs
    }

    #[cfg(any(target_os = "linux", windows))]
    #[must_use]
    pub(crate) fn shadow(&self) -> Self {
        Self {
            fs: Arc::clone(&self.fs),
            memory_locks: None,
            config: self.config.clone(),
            config_digest: self.config_digest,
            previous_config_digest: None,
            registry: self.registry.clone(),
        }
    }

    pub(crate) fn fs_arc(&self) -> Arc<VaultFs> {
        Arc::clone(&self.fs)
    }

    #[must_use]
    pub fn config_seq(&self) -> u64 {
        self.config.config_seq()
    }

    #[must_use]
    pub(crate) fn config(&self) -> &config::VaultConfig {
        &self.config
    }

    #[must_use]
    pub fn has_recovery_phrase(&self) -> bool {
        self.config.has_recovery_recipient()
    }

    #[must_use]
    pub fn unlock_methods(&self) -> Vec<UnlockMethod> {
        self.config
            .recipients_info()
            .unwrap_or_default()
            .into_iter()
            .map(UnlockMethod::from)
            .collect()
    }

    pub async fn enable_platform_unlock(&mut self) -> Result<(), CoreError> {
        if self.effective_policy().no_unlock_shortcuts {
            return Err(CoreError::PolicyForbids(Msg::new(
                "POLICY_NO_UNLOCK_SHORTCUTS",
            )));
        }
        if self.config.has_two_factor_recipient() {
            return Err(CoreError::TwoFactorRequired);
        }
        let (keychain_id, kek) = unlock_methods::generate_and_store_kek()?;
        let mut new_config = self.config.clone();
        if let Err(e) = new_config.set_platform_recipient(&keychain_id, &kek) {
            let _ = unlock_methods::forget_kek(&keychain_id);
            return Err(e.into());
        }
        if let Err(e) = self.commit_config(new_config).await {
            let _ = unlock_methods::forget_kek(&keychain_id);
            return Err(e);
        }
        Ok(())
    }

    pub async fn disable_platform_unlock(&mut self) -> Result<(), CoreError> {
        let keychain_id = self
            .config
            .recipients_info()?
            .into_iter()
            .find_map(|info| match info {
                config::RecipientInfo::Platform { keychain_id } => Some(keychain_id),
                _ => None,
            });
        let mut new_config = self.config.clone();
        new_config.remove_recipient_by_kid(config::PLATFORM_KID)?;
        self.commit_config(new_config).await?;
        if let Some(keychain_id) = keychain_id {
            let _ = unlock_methods::forget_kek(&keychain_id);
        }
        Ok(())
    }

    pub async fn enable_fido2_unlock(
        &mut self,
        authenticator: &dyn Fido2Authenticator,
        rp_id: &str,
        pin: Option<&str>,
    ) -> Result<(), CoreError> {
        if self.config.has_two_factor_recipient() {
            return Err(CoreError::TwoFactorRequired);
        }
        let (credential_id, salt, hmac_secret) =
            unlock_methods::register_fido2(authenticator, rp_id, pin)?;
        let mut new_config = self.config.clone();
        new_config.set_fido2_recipient(&credential_id, &salt, rp_id, &hmac_secret)?;
        self.commit_config(new_config).await
    }

    #[must_use]
    pub fn has_two_factor(&self) -> bool {
        self.config.has_two_factor_recipient()
    }

    pub async fn enable_two_factor(
        &mut self,
        password: &[u8],
        second: &Fido2Auth<'_>,
        rp_id: &str,
    ) -> Result<(), CoreError> {
        if self.config.has_password_recipient() {
            off_worker(|| self.config.check_password(password))?;
        }
        let platform_keychain_id =
            self.config
                .recipients_info()?
                .into_iter()
                .find_map(|info| match info {
                    config::RecipientInfo::Platform { keychain_id } => Some(keychain_id),
                    _ => None,
                });
        let (credential_id, salt, hmac_secret) =
            unlock_methods::register_fido2(second.authenticator, rp_id, second.pin)?;
        let mut new_config = self.config.clone();
        off_worker(|| {
            new_config.set_two_factor_recipient(
                password,
                KdfParams::sensitive(),
                &credential_id,
                &salt,
                rp_id,
                &hmac_secret,
            )
        })?;
        if platform_keychain_id.is_some() {
            new_config.remove_recipient_by_kid(config::PLATFORM_KID)?;
        }
        self.commit_config(new_config).await?;
        if let Some(keychain_id) = platform_keychain_id {
            let _ = unlock_methods::forget_kek(&keychain_id);
        }
        Ok(())
    }

    pub fn two_factor_key_secret(&self, second: &Fido2Auth<'_>) -> Result<Secret<32>, CoreError> {
        let Some((credential_id, salt, rp_id)) = two_factor_params(&self.config.recipients_info()?)
        else {
            return Err(FormatError::NoSuchRecipient.into());
        };
        second
            .authenticator
            .authenticate(&rp_id, &credential_id, &salt, second.pin)
            .map(Secret::new)
            .map_err(|e| CoreError::Fido2(e.msg()))
    }

    pub async fn disable_two_factor(&mut self, new_password: &[u8]) -> Result<(), CoreError> {
        check_new_password(new_password)?;
        let mut new_config = self.config.clone();
        off_worker(|| new_config.remove_two_factor(new_password, KdfParams::sensitive()))?;
        self.commit_config(new_config).await
    }

    pub async fn change_password_two_factor(
        &mut self,
        current_password: &[u8],
        new_password: &[u8],
        second: &Fido2Auth<'_>,
    ) -> Result<(), CoreError> {
        check_new_password(new_password)?;
        let Some((credential_id, salt, rp_id)) = two_factor_params(&self.config.recipients_info()?)
        else {
            return Err(FormatError::NoSuchRecipient.into());
        };
        let secret = Secret::new(
            second
                .authenticator
                .authenticate(&rp_id, &credential_id, &salt, second.pin)
                .map_err(|e| CoreError::Fido2(e.msg()))?,
        );
        let mut new_config = self.config.clone();
        off_worker(|| {
            self.config
                .check_credential(&Credential::PasswordAndFido2 {
                    password: current_password,
                    fido2_secret: &secret,
                })?;
            new_config.set_two_factor_recipient(
                new_password,
                KdfParams::sensitive(),
                &credential_id,
                &salt,
                &rp_id,
                &secret,
            )
        })?;
        self.commit_config(new_config).await
    }

    pub async fn disable_fido2_unlock(&mut self) -> Result<(), CoreError> {
        let mut new_config = self.config.clone();
        new_config.remove_recipient_by_kid(config::FIDO2_KID)?;
        self.commit_config(new_config).await
    }

    #[must_use]
    pub fn list_members(&self) -> Vec<MemberInfo> {
        self.config
            .recipients_info()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|info| match info {
                config::RecipientInfo::Member { name, public_key } => {
                    Some(MemberInfo { name, public_key })
                }
                _ => None,
            })
            .collect()
    }

    pub async fn add_member(
        &mut self,
        member_public_key: &[u8; 32],
        name: &str,
    ) -> Result<(), CoreError> {
        let mut new_config = self.config.clone();
        new_config.set_member_recipient(member_public_key, name)?;
        self.commit_config(new_config).await
    }

    pub async fn remove_member(
        &mut self,
        name: &str,
        auth: &RemoveMemberAuth<'_>,
    ) -> Result<RemoveMemberOutcome, CoreError> {
        let infos = self.config.recipients_info()?;
        let kids: Vec<String> = infos
            .iter()
            .filter_map(|info| match info {
                config::RecipientInfo::Member {
                    name: member_name,
                    public_key,
                } if member_name == name => Some(config::member_kid(public_key)),
                _ => None,
            })
            .collect();
        if kids.is_empty() {
            return Err(FormatError::NoSuchRecipient.into());
        }

        if auth.fido2.is_some() && !self.config.has_two_factor_recipient() {
            self.config.check_password(auth.password)?;
        }
        let fido2_secret = match (
            &auth.fido2,
            infos.iter().find_map(|i| match i {
                config::RecipientInfo::Fido2 {
                    credential_id,
                    salt,
                    rp_id,
                }
                | config::RecipientInfo::TwoFactor {
                    credential_id,
                    salt,
                    rp_id,
                } => Some((credential_id, salt, rp_id)),
                _ => None,
            }),
        ) {
            (Some(fido2), Some((credential_id, salt, rp_id))) => Some(Secret::new(
                fido2
                    .authenticator
                    .authenticate(rp_id, credential_id, salt, fido2.pin)
                    .map_err(|e| CoreError::Fido2(e.msg()))?,
            )),
            _ => None,
        };
        let platform_keychain_id = infos.iter().find_map(|i| match i {
            config::RecipientInfo::Platform { keychain_id } => Some(*keychain_id),
            _ => None,
        });
        let platform_kek =
            platform_keychain_id.and_then(|id| unlock_methods::load_kek(&id).ok().flatten());
        let new_recovery = if self.config.has_recovery_recipient() {
            Some(RecoveryPhrase::generate().map_err(FormatError::from)?)
        } else {
            None
        };
        let new_recovery_entropy = new_recovery.as_ref().map(RecoveryPhrase::entropy);

        let mut new_config = self.config.clone();
        for kid in &kids {
            new_config.remove_recipient_by_kid(kid)?;
        }
        new_config.rotate_seed()?;
        let report = new_config.rotate_cek(&config::RekeyInputs {
            password: Some(auth.password),
            new_recovery_entropy: new_recovery_entropy.as_ref(),
            fido2_secret: fido2_secret.as_ref(),
            platform_kek: platform_kek.as_ref(),
        })?;
        self.commit_config(new_config).await?;

        if report.dropped.contains(&config::DroppedRecipient::Platform)
            && let Some(id) = platform_keychain_id
        {
            let _ = unlock_methods::forget_kek(&id);
        }
        Ok(RemoveMemberOutcome {
            new_recovery_phrase: new_recovery,
            dropped: report.dropped,
        })
    }

    #[must_use]
    pub fn has_inbox(&self) -> bool {
        self.config.has_inbox()
    }

    #[must_use]
    pub fn inbox_public_key(&self) -> Option<[u8; 32]> {
        self.config.inbox_public_key()
    }

    pub async fn enable_inbox(&mut self) -> Result<[u8; 32], CoreError> {
        let mut new_config = self.config.clone();
        let public_key = new_config.enable_inbox()?;
        self.commit_config(new_config).await?;
        Ok(public_key)
    }

    pub async fn disable_inbox(&mut self) -> Result<(), CoreError> {
        let mut new_config = self.config.clone();
        new_config.disable_inbox();
        self.commit_config(new_config).await
    }

    pub async fn list_inbox(&self) -> Result<Vec<crate::inbox::InboxListing>, CoreError> {
        crate::inbox::list_raw(self.fs.backend()).await
    }

    pub async fn collect_inbox(
        &self,
        limits: crate::inbox::CollectLimits,
    ) -> Result<crate::inbox::CollectReport, CoreError> {
        let result = async {
            let inbox_keypair = self.config.inbox_keypair()?;
            let backend = self.fs.backend();
            let raw = crate::inbox::list_raw(backend).await?;

            let mut report = crate::inbox::CollectReport::default();
            let mut total_collected: u64 = 0;
            for listing in raw {
                let deposit_path =
                    ObjPath::new(crate::inbox::INBOX_DIR)?.join(&listing.deposit_name)?;
                if total_collected >= limits.max_total_bytes {
                    report.skipped.push(crate::inbox::SkippedDeposit {
                        deposit_name: listing.deposit_name,
                        reason: Msg::new("INBOX_BUDGET_REACHED").wire(),
                    });
                    continue;
                }
                let remaining_budget = limits
                    .max_total_bytes
                    .saturating_sub(total_collected)
                    .min(limits.max_deposit_bytes);
                match crate::inbox::collect_one(
                    &self.fs,
                    &self.config,
                    backend,
                    &deposit_path,
                    &inbox_keypair,
                    remaining_budget,
                )
                .await
                {
                    Ok(mut collected) => {
                        collected.deposit_name = listing.deposit_name;
                        total_collected += collected.bytes;
                        if let Err(e) = backend.delete(&deposit_path).await {
                            report.skipped.push(crate::inbox::SkippedDeposit {
                                deposit_name: collected.deposit_name.clone(),
                                reason: Msg::new("INBOX_NOT_REMOVED").arg(&e).wire(),
                            });
                        }
                        report.collected.push(collected);
                    }
                    Err(reason) => {
                        report.skipped.push(crate::inbox::SkippedDeposit {
                            deposit_name: listing.deposit_name,
                            reason: reason.wire(),
                        });
                    }
                }
            }
            Ok(report)
        }
        .await;
        self.settled(result).await
    }

    pub async fn set_recovery_phrase(
        &mut self,
        replace: bool,
    ) -> Result<RecoveryPhrase, CoreError> {
        if !replace && self.config.has_recovery_recipient() {
            return Err(CoreError::RecoveryPhraseAlreadyPresent);
        }
        let phrase = RecoveryPhrase::generate().map_err(FormatError::from)?;
        let mut new_config = self.config.clone();
        new_config.set_recovery_seed(&phrase.entropy())?;
        self.commit_config(new_config).await?;
        Ok(phrase)
    }

    pub fn lock_memory(&mut self) -> crate::memlock::MemoryLockReport {
        let locks = crate::memlock::MemoryLocks::of(&self.config);
        let report = locks.report();
        self.memory_locks = Some(locks);
        report
    }

    #[must_use]
    pub fn profile(&self) -> FormatProfile {
        self.config.profile()
    }

    #[must_use]
    pub fn decoy_files(&self) -> bool {
        self.config.decoy_entries()
    }

    #[must_use]
    pub fn policy(&self) -> &VaultPolicy {
        self.config.policy()
    }

    #[must_use]
    pub fn effective_policy(&self) -> VaultPolicy {
        self.config.policy().effective()
    }

    pub async fn set_policy(
        &mut self,
        policy: VaultPolicy,
        reauth: Option<&Credential<'_>>,
    ) -> Result<(), CoreError> {
        if self.config.policy().weakened_by(&policy) {
            let credential = reauth.ok_or(CoreError::PolicyReauthRequired)?;
            let bytes = fetch_vault_uvf(self.fs.backend()).await?;
            off_worker(|| config::open(&bytes, credential))?;
        }
        let mut new_config = self.config.clone();
        new_config.set_policy(policy)?;
        let mut stale_keychain_id = None;
        if new_config.policy().effective().no_unlock_shortcuts {
            stale_keychain_id =
                new_config
                    .recipients_info()?
                    .into_iter()
                    .find_map(|info| match info {
                        config::RecipientInfo::Platform { keychain_id } => Some(keychain_id),
                        _ => None,
                    });
            if stale_keychain_id.is_some() {
                new_config.remove_recipient_by_kid(config::PLATFORM_KID)?;
            }
        }
        self.commit_config(new_config).await?;
        if let Some(keychain_id) = stale_keychain_id {
            let _ = unlock_methods::forget_kek(&keychain_id);
        }
        Ok(())
    }

    pub fn verify_recovery_phrase(&self, phrase: &RecoveryPhrase) -> Result<(), CoreError> {
        let entropy = phrase.entropy();
        Ok(self
            .config
            .check_credential(&Credential::RecoverySeed(&entropy))?)
    }

    pub async fn change_password(&mut self, new_password: &[u8]) -> Result<(), CoreError> {
        if self.config.has_two_factor_recipient() {
            return Err(CoreError::TwoFactorRequired);
        }
        check_new_password(new_password)?;
        let mut new_config = self.config.clone();
        off_worker(|| new_config.set_password(new_password, KdfParams::sensitive()))?;
        self.commit_config(new_config).await
    }

    pub async fn rotate_seed(&mut self) -> Result<(), CoreError> {
        let mut new_config = self.config.clone();
        new_config.rotate_seed()?;
        self.commit_config(new_config).await
    }

    async fn commit_config(
        &mut self,
        new_config: Box<config::VaultConfig>,
    ) -> Result<(), CoreError> {
        let path = ObjPath::root().join("vault.uvf")?;
        let current = self.fs.backend().get(&path).await?;
        let current = digest_of(&current);
        if current != self.config_digest && Some(current) != self.previous_config_digest {
            return Err(CoreError::ConfigChangedElsewhere);
        }
        let mut new_config = new_config;
        new_config.bump_config_seq();
        let bytes = new_config.serialize()?;
        self.fs.backend().put(&path, &bytes).await?;
        if let Some(data_dir) = crate::registry::global_data_dir() {
            let vault_id =
                cu_format::keys::root_dir_id(new_config.initial_seed(), new_config.kdf_salt())?;
            let _ =
                crate::config_head::remember_written(data_dir, &vault_id, new_config.config_seq());
        }
        self.previous_config_digest = Some(self.config_digest);
        self.config_digest = digest_of(&bytes);
        let relock = self.memory_locks.take().is_some();
        self.config = new_config;
        if relock {
            self.memory_locks = Some(crate::memlock::MemoryLocks::of(&self.config));
        }
        if let Some(registry) = &self.registry {
            registry.set_config(&self.config);
        }
        Ok(())
    }

    pub async fn list(&self, vault_dir: &str) -> Result<Vec<Entry>, CoreError> {
        Ok(self.fs.list(&self.config, vault_dir).await?)
    }

    pub async fn list_detailed(&self, vault_dir: &str) -> Result<Vec<DetailedEntry>, CoreError> {
        Ok(self.fs.list_detailed(&self.config, vault_dir).await?)
    }

    pub async fn physical_dir(&self, vault_dir: &str) -> Result<String, CoreError> {
        Ok(self.fs.physical_dir(&self.config, vault_dir).await?)
    }

    pub async fn mkdir(&self, vault_path: &str) -> Result<(), CoreError> {
        let result = self.fs.mkdir(&self.config, vault_path).await;
        self.settled(result.map_err(Into::into)).await
    }

    pub async fn import_file(
        &self,
        local_src: &Path,
        vault_dest: &str,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), CoreError> {
        let result = self
            .import_file_inner(local_src, vault_dest, on_bytes)
            .await;
        self.settled(result).await
    }

    async fn import_file_inner(
        &self,
        local_src: &Path,
        vault_dest: &str,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), CoreError> {
        let content_hash = hash_file(local_src).await?;
        let mut file = tokio::fs::File::open(local_src).await?;
        let result = self
            .fs
            .write_file(
                &self.config,
                vault_dest,
                &mut file,
                Some(content_hash),
                on_bytes,
            )
            .await;
        result.map_err(Into::into)
    }

    pub async fn export_file(
        &self,
        vault_src: &str,
        local_dest: &Path,
        on_bytes: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), CoreError> {
        if let Some(parent) = local_dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::io::BufWriter::with_capacity(
            EXPORT_BUFFER,
            tokio::fs::File::create(local_dest).await?,
        );
        self.fs
            .read_file(&self.config, vault_src, &mut file, on_bytes)
            .await?;
        Ok(())
    }

    pub async fn remove_file(&self, vault_path: &str) -> Result<(), CoreError> {
        let result = self.fs.remove_file(&self.config, vault_path).await;
        self.settled(result.map_err(Into::into)).await
    }

    pub async fn remove_dir(&self, vault_path: &str) -> Result<(), CoreError> {
        let result = self.fs.remove_dir(&self.config, vault_path).await;
        self.settled(result.map_err(Into::into)).await
    }

    pub async fn import_tree(
        &self,
        local_src: &Path,
        vault_dest: &str,
        on_event: &mut (dyn FnMut(ProgressEvent) + Send),
    ) -> Result<(), CoreError> {
        self.fs.begin_dirmeta_batch();
        let result = self
            .import_tree_inner(local_src, vault_dest, on_event)
            .await;
        self.fs.end_dirmeta_batch(&self.config).await;
        self.settled(result).await
    }

    pub async fn import_many(
        &self,
        items: &[(std::path::PathBuf, String)],
        on_event: &mut (dyn FnMut(usize, ProgressEvent) + Send),
    ) -> Vec<Result<(), CoreError>> {
        self.fs.begin_dirmeta_batch();
        let mut results = Vec::with_capacity(items.len());
        for (index, (local_src, vault_dest)) in items.iter().enumerate() {
            let mut item_event = |event: ProgressEvent| on_event(index, event);
            results.push(
                self.import_tree_inner(local_src, vault_dest, &mut item_event)
                    .await,
            );
        }
        self.fs.end_dirmeta_batch(&self.config).await;
        self.flush_registry().await;
        results
    }

    fn import_tree_inner<'a>(
        &'a self,
        local_src: &'a Path,
        vault_dest: &'a str,
        on_event: &'a mut (dyn FnMut(ProgressEvent) + Send),
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            let meta = tokio::fs::metadata(local_src).await?;
            if meta.is_file() {
                on_event(ProgressEvent::File {
                    vault_path: vault_dest.to_string(),
                });
                let mut on_bytes = |count| on_event(ProgressEvent::Bytes { count });
                return self
                    .import_file_inner(local_src, vault_dest, &mut on_bytes)
                    .await;
            }
            self.fs.mkdir(&self.config, vault_dest).await?;
            let mut entries = tokio::fs::read_dir(local_src).await?;
            while let Some(entry) = entries.next_entry().await? {
                let os_name = entry.file_name();
                let name = os_name
                    .to_str()
                    .ok_or_else(|| CoreError::InvalidName(os_name.display().to_string()))?;
                let child_local = entry.path();
                let child_vault = join_vault_path(vault_dest, name);
                self.import_tree_inner(&child_local, &child_vault, on_event)
                    .await?;
            }
            Ok(())
        })
    }

    pub fn export_tree<'a>(
        &'a self,
        vault_src: &'a str,
        local_dest: &'a Path,
        on_event: &'a mut (dyn FnMut(ProgressEvent) + Send),
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), CoreError>> + Send + 'a>>
    {
        Box::pin(async move {
            match self.fs.list(&self.config, vault_src).await {
                Ok(children) => {
                    tokio::fs::create_dir_all(local_dest).await?;
                    for entry in children {
                        let child_vault = join_vault_path(vault_src, &entry.name);
                        let child_local = join_exported_name(local_dest, &entry.name)?;
                        self.export_tree(&child_vault, &child_local, on_event)
                            .await?;
                    }
                    Ok(())
                }
                Err(cu_vfs::VfsError::NotFound) => {
                    on_event(ProgressEvent::File {
                        vault_path: vault_src.to_string(),
                    });
                    let mut on_bytes = |count| on_event(ProgressEvent::Bytes { count });
                    self.export_file(vault_src, local_dest, &mut on_bytes).await
                }
                Err(e) => Err(e.into()),
            }
        })
    }
}

async fn fetch_vault_uvf(backend: &dyn StorageBackend) -> Result<Vec<u8>, CoreError> {
    match backend.get(&ObjPath::root().join("vault.uvf")?).await {
        Ok(bytes) => Ok(bytes),
        Err(BackendError::NotFound) => Err(CoreError::NotAVault),
        Err(e) => Err(e.into()),
    }
}

fn two_factor_params(infos: &[config::RecipientInfo]) -> Option<(Vec<u8>, [u8; 32], String)> {
    infos.iter().find_map(|info| match info {
        config::RecipientInfo::TwoFactor {
            credential_id,
            salt,
            rp_id,
        } => Some((credential_id.clone(), *salt, rp_id.clone())),
        _ => None,
    })
}

fn off_worker<R>(f: impl FnOnce() -> R) -> R {
    use tokio::runtime::{Handle, RuntimeFlavor};
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

fn digest_of(bytes: &[u8]) -> [u8; 32] {
    cu_crypto::hash::Hasher::new().update(bytes).finalize()
}

fn join_exported_name(parent: &Path, name: &str) -> Result<std::path::PathBuf, CoreError> {
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(only)), None) if only == std::ffi::OsStr::new(name) => {
            Ok(parent.join(name))
        }
        _ => Err(CoreError::InvalidName(name.to_string())),
    }
}

pub const MIN_PASSWORD_CHARS: usize = 8;

pub fn check_new_password(password: &[u8]) -> Result<(), CoreError> {
    let chars = String::from_utf8_lossy(password).chars().count();
    if chars < MIN_PASSWORD_CHARS {
        return Err(CoreError::WeakPassword {
            min_chars: MIN_PASSWORD_CHARS,
        });
    }
    Ok(())
}

fn join_vault_path(parent: &str, child: &str) -> String {
    if parent.trim_matches('/').is_empty() {
        child.to_string()
    } else {
        format!("{}/{child}", parent.trim_matches('/'))
    }
}

async fn hash_file(path: &Path) -> Result<[u8; 32], CoreError> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = cu_crypto::hash::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize())
}
