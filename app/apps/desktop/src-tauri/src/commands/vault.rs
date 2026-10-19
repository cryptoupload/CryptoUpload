// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::Msg;
use std::path::PathBuf;

use tauri::State;
use zeroize::Zeroize;

use crate::error::UiError;
use crate::state::{AppState, OpenVault, VaultLocation};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultInfo {
    file_format: String,
    name_format: String,
    kdf_name: String,
    seed_count: usize,
    has_recovery_phrase: bool,
    private_profile: bool,
    decoy_files: bool,
}

impl From<&cu_core::Vault> for VaultInfo {
    fn from(vault: &cu_core::Vault) -> Self {
        Self {
            file_format: vault.file_format().to_string(),
            name_format: vault.name_format().to_string(),
            kdf_name: vault.kdf_name().to_string(),
            seed_count: vault.seed_count(),
            has_recovery_phrase: vault.has_recovery_phrase(),
            private_profile: vault.profile() == cu_core::FormatProfile::Private,
            decoy_files: vault.decoy_files(),
        }
    }
}

fn location_key(location: &VaultLocation) -> String {
    match location {
        VaultLocation::Local(path) => path.display().to_string(),
        VaultLocation::Remote(name) => format!("remote:{name}"),
    }
}

async fn configured_bandwidth_limit_kib(state: &State<'_, AppState>) -> Option<u32> {
    let kib = state.settings.lock().await.bandwidth_limit_kib;
    (kib > 0).then_some(kib)
}

fn vault_ref(location: &VaultLocation) -> cu_core::VaultRef {
    match location {
        VaultLocation::Local(path) => cu_core::VaultRef::Local(path.clone()),
        VaultLocation::Remote(name) => cu_core::VaultRef::Remote(name.clone()),
    }
}

async fn admit(
    state: &State<'_, AppState>,
    location: &VaultLocation,
    method: &str,
    result: Result<cu_core::Vault, cu_core::CoreError>,
    allow_remote_session: Option<bool>,
) -> Result<cu_core::Vault, UiError> {
    let vault = match result {
        Ok(vault) => vault,
        Err(e) => {
            if e.is_decrypt_failure() && state.settings.lock().await.security.failed_unlock_notice {
                cu_core::attempts::record_failed_unlock(
                    &state.data_dir,
                    &vault_ref(location),
                    method,
                );
            }
            return Err(e.into());
        }
    };
    let mut acknowledged = std::collections::BTreeSet::new();
    if vault.effective_policy().block_remote_sessions {
        let evidence = crate::guard::remote_evidence().await;
        if !evidence.is_empty() && allow_remote_session != Some(true) {
            return Err(UiError::coded(
                &Msg::new("REMOTE_SESSION")
                    .arg(evidence.into_iter().collect::<Vec<_>>().join(", ")),
            ));
        }
        acknowledged = evidence;
    }
    *state
        .remote_acknowledged
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = acknowledged;
    Ok(vault)
}

async fn store_open_vault(
    state: &State<'_, AppState>,
    vault: cu_core::Vault,
    location: VaultLocation,
) -> VaultInfo {
    let mut vault = vault;
    if state.settings.lock().await.security.lock_memory {
        state.lock_vault_memory(&mut vault);
    }
    let info = VaultInfo::from(&vault);
    state
        .store_vault(OpenVault {
            vault,
            location: location.clone(),
        })
        .await;

    let mut settings = state.settings.lock().await;
    crate::settings::remember_vault(&mut settings, &location_key(&location));
    if let Err(e) = crate::settings::save(&state.data_dir, &settings) {
        eprintln!("cannot save ui.json: {e}");
    }

    info
}

fn creation_of(
    private_profile: Option<bool>,
    decoy_files: Option<bool>,
    readme_text: Option<String>,
) -> cu_core::VaultCreation {
    let private = private_profile == Some(true);
    cu_core::VaultCreation {
        profile: if private {
            cu_core::FormatProfile::Private
        } else {
            cu_core::FormatProfile::Standard
        },
        decoy_files: private && decoy_files == Some(true),
        readme: readme_text.filter(|t| !t.trim().is_empty()),
    }
}

#[tauri::command]
pub async fn vault_create(
    state: State<'_, AppState>,
    root: PathBuf,
    password: String,
    private_profile: Option<bool>,
    decoy_files: Option<bool>,
    readme_text: Option<String>,
) -> Result<VaultInfo, UiError> {
    let bandwidth_limit_kib = configured_bandwidth_limit_kib(&state).await;
    let mut password = password;
    let result = cu_core::Vault::create_local_in_profile(
        &root,
        password.as_bytes(),
        bandwidth_limit_kib,
        creation_of(private_profile, decoy_files, readme_text),
    )
    .await;
    password.zeroize();
    let vault = result?;
    Ok(store_open_vault(&state, vault, VaultLocation::Local(root)).await)
}

#[tauri::command]
pub async fn vault_unlock(
    state: State<'_, AppState>,
    root: PathBuf,
    password: String,
    allow_remote_session: Option<bool>,
    key_pin: Option<String>,
) -> Result<VaultInfo, UiError> {
    let bandwidth_limit_kib = configured_bandwidth_limit_kib(&state).await;
    let mut password = password;
    let credential = cu_core::Credential::Password(password.as_bytes());
    let mut key_pin = key_pin;
    let mut result =
        cu_core::Vault::open_local_with_bandwidth(&root, &credential, bandwidth_limit_kib).await;
    if matches!(result, Err(cu_core::CoreError::TwoFactorRequired)) {
        let authenticator = cu_core::RealFido2Authenticator::new();
        result = cu_core::Vault::open_local_two_factor(
            &root,
            password.as_bytes(),
            &cu_core::Fido2Auth {
                authenticator: &authenticator,
                pin: key_pin.as_deref(),
            },
            bandwidth_limit_kib,
        )
        .await;
    }
    if let Some(p) = key_pin.as_mut() {
        p.zeroize();
    }
    password.zeroize();
    let location = VaultLocation::Local(root);
    let vault = admit(&state, &location, "password", result, allow_remote_session).await?;
    Ok(store_open_vault(&state, vault, location).await)
}

#[tauri::command]
pub async fn vault_unlock_recovery(
    state: State<'_, AppState>,
    root: PathBuf,
    words: String,
    allow_remote_session: Option<bool>,
) -> Result<VaultInfo, UiError> {
    let mut words = words;
    let entropy = cu_core::recovery_shares::recovery_entropy_from_text(&words);
    words.zeroize();
    let entropy = entropy?;
    let credential = cu_core::Credential::RecoverySeed(&entropy);
    let bandwidth_limit_kib = configured_bandwidth_limit_kib(&state).await;
    let result =
        cu_core::Vault::open_local_with_bandwidth(&root, &credential, bandwidth_limit_kib).await;
    let location = VaultLocation::Local(root);
    let vault = admit(&state, &location, "recovery", result, allow_remote_session).await?;
    Ok(store_open_vault(&state, vault, location).await)
}

#[tauri::command]
pub async fn vault_generate_recovery_phrase(
    state: State<'_, AppState>,
    replace: bool,
) -> Result<Vec<String>, UiError> {
    let mut guard = state.vault.lock().await;
    let open_vault = guard.as_mut().ok_or_else(UiError::no_vault)?;
    let phrase = open_vault.vault.set_recovery_phrase(replace).await?;
    Ok(phrase.words().map(str::to_string).collect())
}

#[tauri::command]
pub async fn vault_change_password(
    state: State<'_, AppState>,
    mut new_password: String,
) -> Result<(), UiError> {
    let result = change_password_inner(&state, &new_password).await;
    new_password.zeroize();
    result
}

async fn change_password_inner(
    state: &State<'_, AppState>,
    new_password: &str,
) -> Result<(), UiError> {
    if new_password.is_empty() {
        return Err(UiError::coded(&Msg::new("PASSWORD_EMPTY")));
    }
    let mut guard = state.vault.lock().await;
    let open_vault = guard.as_mut().ok_or_else(UiError::no_vault)?;
    open_vault
        .vault
        .change_password(new_password.as_bytes())
        .await?;
    Ok(())
}

async fn find_remote_config(
    state: &AppState,
    name: &str,
) -> Result<(cu_core::RemoteConfig, cu_core::RefreshTokenSink), UiError> {
    let guard = state.unlocked_remotes.lock().await;
    let unlocked = guard
        .get(name)
        .ok_or_else(|| UiError::coded(&Msg::new("REMOTE_NOT_UNLOCKED").arg(name)))?;
    let sink = cu_core::remote::persisting_refresh_sink(
        state.secrets_file_path(),
        name.to_string(),
        unlocked.password.as_bytes().to_vec(),
    );
    Ok((unlocked.config.clone(), sink))
}

#[tauri::command]
pub async fn vault_create_remote(
    state: State<'_, AppState>,
    remote_name: String,
    password: String,
    private_profile: Option<bool>,
    decoy_files: Option<bool>,
    readme_text: Option<String>,
) -> Result<VaultInfo, UiError> {
    let (cfg, refresh_sink) = find_remote_config(&state, &remote_name).await?;
    let remote_state_db =
        cu_core::resume::remote_state_db_path(&state.data_dir, &cu_core::remote::remote_id(&cfg));
    if let Some(parent) = remote_state_db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let bandwidth_limit_kib = configured_bandwidth_limit_kib(&state).await;
    let mut password = password;
    let result = cu_core::Vault::create_remote_in_profile(
        &cfg,
        &remote_state_db,
        password.as_bytes(),
        bandwidth_limit_kib,
        Some(refresh_sink),
        creation_of(private_profile, decoy_files, readme_text),
    )
    .await;
    password.zeroize();
    let vault = result?;
    Ok(store_open_vault(&state, vault, VaultLocation::Remote(remote_name)).await)
}

#[tauri::command]
pub async fn vault_unlock_remote(
    state: State<'_, AppState>,
    remote_name: String,
    password: String,
    allow_remote_session: Option<bool>,
    key_pin: Option<String>,
) -> Result<VaultInfo, UiError> {
    let (cfg, refresh_sink) = find_remote_config(&state, &remote_name).await?;
    let remote_state_db =
        cu_core::resume::remote_state_db_path(&state.data_dir, &cu_core::remote::remote_id(&cfg));
    if let Some(parent) = remote_state_db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let bandwidth_limit_kib = configured_bandwidth_limit_kib(&state).await;
    let mut password = password;
    let credential = cu_core::Credential::Password(password.as_bytes());
    let mut key_pin = key_pin;
    let mut result = cu_core::Vault::open_remote_with_refresh_sink(
        &cfg,
        &remote_state_db,
        &credential,
        bandwidth_limit_kib,
        Some(refresh_sink.clone()),
    )
    .await;
    if matches!(result, Err(cu_core::CoreError::TwoFactorRequired)) {
        let authenticator = cu_core::RealFido2Authenticator::new();
        result = cu_core::Vault::open_remote_two_factor(
            &cfg,
            &remote_state_db,
            password.as_bytes(),
            &cu_core::Fido2Auth {
                authenticator: &authenticator,
                pin: key_pin.as_deref(),
            },
            bandwidth_limit_kib,
            Some(refresh_sink),
        )
        .await;
    }
    if let Some(p) = key_pin.as_mut() {
        p.zeroize();
    }
    password.zeroize();
    let location = VaultLocation::Remote(remote_name);
    let vault = admit(&state, &location, "password", result, allow_remote_session).await?;
    Ok(store_open_vault(&state, vault, location).await)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum UnlockMethodDto {
    Password,
    RecoverySeed,
    Fido2 { rp_id: String },
    Platform,
    TwoFactor,
    Member { name: String, public_key: String },
    Unknown { kid: String },
}

impl From<cu_core::UnlockMethod> for UnlockMethodDto {
    fn from(method: cu_core::UnlockMethod) -> Self {
        match method {
            cu_core::UnlockMethod::Password => Self::Password,
            cu_core::UnlockMethod::RecoverySeed => Self::RecoverySeed,
            cu_core::UnlockMethod::Fido2 { rp_id } => Self::Fido2 { rp_id },
            cu_core::UnlockMethod::Platform => Self::Platform,
            cu_core::UnlockMethod::TwoFactor { .. } => Self::TwoFactor,
            cu_core::UnlockMethod::Member { name, public_key } => Self::Member {
                name,
                public_key: encode_member_public_key(&public_key),
            },
            cu_core::UnlockMethod::Unknown(kid) => Self::Unknown { kid },
        }
    }
}

#[tauri::command]
pub async fn vault_peek_unlock_methods(root: PathBuf) -> Result<Vec<UnlockMethodDto>, UiError> {
    let methods = cu_core::Vault::peek_unlock_methods(&root).await?;
    Ok(methods.into_iter().map(UnlockMethodDto::from).collect())
}

#[tauri::command]
pub async fn vault_unlock_methods(
    state: State<'_, AppState>,
) -> Result<Vec<UnlockMethodDto>, UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    Ok(open_vault
        .vault
        .unlock_methods()
        .into_iter()
        .map(UnlockMethodDto::from)
        .collect())
}

#[tauri::command]
pub async fn vault_unlock_platform(
    state: State<'_, AppState>,
    root: PathBuf,
    allow_remote_session: Option<bool>,
) -> Result<VaultInfo, UiError> {
    let result = cu_core::Vault::open_local_with_platform_key(&root).await;
    let location = VaultLocation::Local(root);
    let vault = admit(&state, &location, "platform", result, allow_remote_session).await?;
    Ok(store_open_vault(&state, vault, location).await)
}

#[tauri::command]
pub async fn vault_unlock_fido2(
    state: State<'_, AppState>,
    root: PathBuf,
    pin: Option<String>,
    allow_remote_session: Option<bool>,
) -> Result<VaultInfo, UiError> {
    let mut pin = pin;
    let authenticator = cu_core::RealFido2Authenticator::new();
    let result = cu_core::Vault::open_local_with_fido2(&root, &authenticator, pin.as_deref()).await;
    if let Some(p) = pin.as_mut() {
        p.zeroize();
    }
    let location = VaultLocation::Local(root);
    let vault = admit(&state, &location, "fido2", result, allow_remote_session).await?;
    Ok(store_open_vault(&state, vault, location).await)
}

#[tauri::command]
pub async fn recovery_split(
    state: State<'_, AppState>,
    words: String,
    threshold: u8,
    count: u8,
) -> Result<Vec<String>, UiError> {
    let mut words = words;
    let phrase = cu_core::RecoveryPhrase::parse(words.trim()).map_err(|e| UiError::from_msg(&e));
    words.zeroize();
    let phrase = phrase?;
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    open_vault
        .vault
        .verify_recovery_phrase(&phrase)
        .map_err(|e| {
            if e.is_decrypt_failure() {
                UiError::coded(&Msg::new("RECOVERY_WORDS_NOT_OF_VAULT"))
            } else {
                UiError::from(e)
            }
        })?;
    let pieces = cu_core::recovery_shares::split_recovery_phrase(&phrase, threshold, count)?;
    Ok(pieces.iter().map(|piece| piece.to_string()).collect())
}

#[tauri::command]
pub async fn vault_enable_two_factor(
    state: State<'_, AppState>,
    password: String,
    pin: Option<String>,
) -> Result<(), UiError> {
    let (mut password, mut pin) = (password, pin);
    let authenticator = cu_core::RealFido2Authenticator::new();
    let mut guard = state.vault.lock().await;
    let result = match guard.as_mut() {
        Some(open_vault) => open_vault
            .vault
            .enable_two_factor(
                password.as_bytes(),
                &cu_core::Fido2Auth {
                    authenticator: &authenticator,
                    pin: pin.as_deref(),
                },
                cu_core::FIDO2_RP_ID,
            )
            .await
            .map_err(|e| {
                if e.is_decrypt_failure() {
                    UiError::coded(&Msg::new("TWO_FACTOR_WRONG_PASSWORD"))
                } else {
                    UiError::from(e)
                }
            }),
        None => Err(UiError::no_vault()),
    };
    password.zeroize();
    if let Some(p) = pin.as_mut() {
        p.zeroize();
    }
    result
}

#[tauri::command]
pub async fn vault_disable_two_factor(
    state: State<'_, AppState>,
    new_password: String,
) -> Result<(), UiError> {
    let mut new_password = new_password;
    let mut guard = state.vault.lock().await;
    let result = if new_password.is_empty() {
        Err(UiError::coded(&Msg::new("PASSWORD_EMPTY")))
    } else {
        match guard.as_mut() {
            Some(open_vault) => open_vault
                .vault
                .disable_two_factor(new_password.as_bytes())
                .await
                .map_err(UiError::from),
            None => Err(UiError::no_vault()),
        }
    };
    new_password.zeroize();
    result
}

#[tauri::command]
pub async fn vault_change_password_two_factor(
    state: State<'_, AppState>,
    current_password: String,
    new_password: String,
    pin: Option<String>,
) -> Result<(), UiError> {
    let (mut current_password, mut new_password, mut pin) = (current_password, new_password, pin);
    let authenticator = cu_core::RealFido2Authenticator::new();
    let mut guard = state.vault.lock().await;
    let result = if new_password.is_empty() {
        Err(UiError::coded(&Msg::new("PASSWORD_EMPTY")))
    } else {
        match guard.as_mut() {
            Some(open_vault) => open_vault
                .vault
                .change_password_two_factor(
                    current_password.as_bytes(),
                    new_password.as_bytes(),
                    &cu_core::Fido2Auth {
                        authenticator: &authenticator,
                        pin: pin.as_deref(),
                    },
                )
                .await
                .map_err(|e| {
                    if e.is_decrypt_failure() {
                        UiError::coded(&Msg::new("PASSWORD_CHANGE_MISMATCH"))
                    } else {
                        UiError::from(e)
                    }
                }),
            None => Err(UiError::no_vault()),
        }
    };
    current_password.zeroize();
    new_password.zeroize();
    if let Some(p) = pin.as_mut() {
        p.zeroize();
    }
    result
}

#[tauri::command]
pub async fn unlock_attempts_take(
    state: State<'_, AppState>,
) -> Result<Vec<cu_core::attempts::FailedUnlock>, UiError> {
    if !state.settings.lock().await.security.failed_unlock_notice {
        return Ok(Vec::new());
    }
    let guard = state.vault.lock().await;
    Ok(guard.as_ref().map_or_else(Vec::new, |open| {
        cu_core::attempts::take_failed_unlocks(&state.data_dir, &vault_ref(&open.location))
    }))
}

#[tauri::command]
pub async fn vault_enable_platform_unlock(state: State<'_, AppState>) -> Result<(), UiError> {
    let mut guard = state.vault.lock().await;
    let open_vault = guard.as_mut().ok_or_else(UiError::no_vault)?;
    open_vault.vault.enable_platform_unlock().await?;
    Ok(())
}

#[tauri::command]
pub async fn vault_disable_platform_unlock(state: State<'_, AppState>) -> Result<(), UiError> {
    let mut guard = state.vault.lock().await;
    let open_vault = guard.as_mut().ok_or_else(UiError::no_vault)?;
    open_vault.vault.disable_platform_unlock().await?;
    Ok(())
}

#[tauri::command]
pub async fn vault_enable_fido2(
    state: State<'_, AppState>,
    pin: Option<String>,
) -> Result<(), UiError> {
    let mut pin = pin;
    let authenticator = cu_core::RealFido2Authenticator::new();
    let mut guard = state.vault.lock().await;
    let result = if let Some(open_vault) = guard.as_mut() {
        open_vault
            .vault
            .enable_fido2_unlock(&authenticator, cu_core::FIDO2_RP_ID, pin.as_deref())
            .await
            .map_err(UiError::from)
    } else {
        Err(UiError::no_vault())
    };
    if let Some(p) = pin.as_mut() {
        p.zeroize();
    }
    result
}

#[tauri::command]
pub async fn vault_disable_fido2(state: State<'_, AppState>) -> Result<(), UiError> {
    let mut guard = state.vault.lock().await;
    let open_vault = guard.as_mut().ok_or_else(UiError::no_vault)?;
    open_vault.vault.disable_fido2_unlock().await?;
    Ok(())
}

fn encode_member_public_key(bytes: &[u8; 32]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(bytes)
}

fn decode_member_public_key(raw: &str) -> Result<[u8; 32], UiError> {
    let bytes = data_encoding::BASE64URL_NOPAD
        .decode(raw.trim().as_bytes())
        .map_err(|_| UiError::coded(&Msg::new("PUBKEY_NOT_BASE64")))?;
    bytes
        .try_into()
        .map_err(|_| UiError::coded(&Msg::new("PUBKEY_NOT_32_BYTES")))
}

fn member_identity_path(state: &AppState) -> PathBuf {
    cu_core::member_identity::identity_path(&state.data_dir)
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub fn member_identity_exists(state: State<'_, AppState>) -> bool {
    cu_core::member_identity::exists(&member_identity_path(&state))
}

pub(crate) async fn run_kdf<T, F>(f: F) -> Result<T, UiError>
where
    F: FnOnce() -> Result<T, UiError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| UiError::coded(&Msg::new("KDF_INTERRUPTED")))?
}

#[tauri::command]
pub async fn member_identity_generate(
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<String, UiError> {
    let mut passphrase = passphrase;
    let path = member_identity_path(&state);
    run_kdf(move || {
        let result = cu_core::member_identity::generate_and_save(&path, passphrase.as_bytes());
        passphrase.zeroize();
        let keypair = result.map_err(|e| UiError::from_msg(&e))?;
        Ok(encode_member_public_key(&keypair.public_bytes()))
    })
    .await
}

#[tauri::command]
pub async fn member_identity_public_key(
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<String, UiError> {
    let mut passphrase = passphrase;
    let path = member_identity_path(&state);
    run_kdf(move || {
        let result = cu_core::member_identity::load(&path, passphrase.as_bytes());
        passphrase.zeroize();
        let keypair = result.map_err(|e| UiError::from_msg(&e))?;
        Ok(encode_member_public_key(&keypair.public_bytes()))
    })
    .await
}

#[tauri::command]
pub async fn vault_unlock_member(
    state: State<'_, AppState>,
    root: PathBuf,
    identity_passphrase: String,
    accept_new_vault: Option<bool>,
    allow_remote_session: Option<bool>,
) -> Result<VaultInfo, UiError> {
    let mut identity_passphrase = identity_passphrase;
    if accept_new_vault == Some(true) {
        let data_dir = cu_core::data_dir()?;
        cu_core::forget_member_vault_identity(&data_dir, &root)?;
    }
    let path = member_identity_path(&state);
    let keypair = run_kdf(move || {
        let result = cu_core::member_identity::load(&path, identity_passphrase.as_bytes());
        identity_passphrase.zeroize();
        result.map_err(|e| UiError::from_msg(&e))
    })
    .await?;
    let vault = match cu_core::Vault::open_local_with_member_key(&root, &keypair).await {
        Ok(vault) => Ok(vault),
        Err(cu_core::CoreError::VaultIdentityChanged) => {
            return Err(UiError::coded(&Msg::new("VAULT_IDENTITY_CHANGED")));
        }
        Err(e) => Err(e),
    };
    let location = VaultLocation::Local(root);
    let vault = admit(&state, &location, "member", vault, allow_remote_session).await?;
    Ok(store_open_vault(&state, vault, location).await)
}

#[tauri::command]
pub fn vault_accept_config_rollback() {
    cu_core::accept_next_config_rollback();
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDto {
    name: String,
    public_key: String,
}

impl From<cu_core::MemberInfo> for MemberDto {
    fn from(info: cu_core::MemberInfo) -> Self {
        Self {
            name: info.name,
            public_key: encode_member_public_key(&info.public_key),
        }
    }
}

#[tauri::command]
pub async fn vault_list_members(state: State<'_, AppState>) -> Result<Vec<MemberDto>, UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    Ok(open_vault
        .vault
        .list_members()
        .into_iter()
        .map(MemberDto::from)
        .collect())
}

#[tauri::command]
pub async fn vault_add_member(
    state: State<'_, AppState>,
    name: String,
    public_key_b64: String,
) -> Result<(), UiError> {
    let public_key = decode_member_public_key(&public_key_b64)?;
    let mut guard = state.vault.lock().await;
    let open_vault = guard.as_mut().ok_or_else(UiError::no_vault)?;
    open_vault.vault.add_member(&public_key, &name).await?;
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveMemberResult {
    new_recovery_phrase: Option<Vec<String>>,
    dropped: Vec<String>,
}

#[tauri::command]
pub async fn vault_remove_member(
    state: State<'_, AppState>,
    name: String,
    password: String,
    use_fido2: bool,
    pin: Option<String>,
) -> Result<RemoveMemberResult, UiError> {
    let mut password = password;
    let mut pin = pin;
    let authenticator = cu_core::RealFido2Authenticator::new();
    let mut guard = state.vault.lock().await;
    let result = if let Some(open_vault) = guard.as_mut() {
        open_vault
            .vault
            .remove_member(
                &name,
                &cu_core::RemoveMemberAuth {
                    password: password.as_bytes(),
                    fido2: use_fido2.then_some(cu_core::Fido2Auth {
                        authenticator: &authenticator,
                        pin: pin.as_deref(),
                    }),
                },
            )
            .await
            .map_err(UiError::from)
    } else {
        Err(UiError::no_vault())
    };
    password.zeroize();
    if let Some(p) = pin.as_mut() {
        p.zeroize();
    }
    let outcome = result?;
    Ok(RemoveMemberResult {
        new_recovery_phrase: outcome
            .new_recovery_phrase
            .map(|phrase| phrase.words().map(str::to_string).collect()),
        dropped: outcome
            .dropped
            .iter()
            .filter_map(|d| match d {
                cu_core::DroppedRecipient::Fido2 => Some("fido2".to_string()),
                cu_core::DroppedRecipient::Platform => Some("platform".to_string()),
                cu_core::DroppedRecipient::Unknown(_) => Some("unknown".to_string()),
                cu_core::DroppedRecipient::Recovery => None,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn vault_lock(state: State<'_, AppState>) -> Result<(), UiError> {
    #[cfg(any(target_os = "linux", windows))]
    {
        let mount_guard = state.mount.lock().await;
        if mount_guard.is_some() {
            return Err(UiError::coded(&Msg::new("VAULT_DRIVE_STILL_MOUNTED")));
        }
    }
    state.clear_vault().await;
    state.unlocked_remotes.lock().await.clear();
    Ok(())
}

#[tauri::command]
pub async fn webview_recycle(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    notice: Option<serde_json::Value>,
) -> Result<bool, UiError> {
    *state
        .lock_notice
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = notice;
    let hidden = state.settings.lock().await.security.hide_from_capture;
    crate::webview_recycle::schedule(&app, hidden);
    Ok(true)
}

#[tauri::command]
pub async fn lock_notice_take(
    state: State<'_, AppState>,
) -> Result<Option<serde_json::Value>, UiError> {
    Ok(state
        .lock_notice
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxListingDto {
    deposit_name: String,
    encrypted_size: Option<u64>,
}

impl From<cu_core::InboxListing> for InboxListingDto {
    fn from(l: cu_core::InboxListing) -> Self {
        Self {
            deposit_name: l.deposit_name,
            encrypted_size: l.encrypted_size,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectedDepositDto {
    vault_path: String,
    original_name: String,
    depositor_label: String,
    bytes: u64,
}

impl From<cu_core::CollectedDeposit> for CollectedDepositDto {
    fn from(c: cu_core::CollectedDeposit) -> Self {
        Self {
            vault_path: c.vault_path,
            original_name: c.original_name,
            depositor_label: c.depositor_label,
            bytes: c.bytes,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedDepositDto {
    deposit_name: String,
    reason: String,
}

impl From<cu_core::SkippedDeposit> for SkippedDepositDto {
    fn from(s: cu_core::SkippedDeposit) -> Self {
        Self {
            deposit_name: s.deposit_name,
            reason: s.reason,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectReportDto {
    collected: Vec<CollectedDepositDto>,
    skipped: Vec<SkippedDepositDto>,
}

impl From<cu_core::CollectReport> for CollectReportDto {
    fn from(r: cu_core::CollectReport) -> Self {
        Self {
            collected: r.collected.into_iter().map(Into::into).collect(),
            skipped: r.skipped.into_iter().map(Into::into).collect(),
        }
    }
}

#[tauri::command]
pub async fn vault_inbox_status(
    state: State<'_, AppState>,
) -> Result<(bool, Option<String>), UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    Ok((
        open_vault.vault.has_inbox(),
        open_vault
            .vault
            .inbox_public_key()
            .map(|k| encode_member_public_key(&k)),
    ))
}

#[tauri::command]
pub async fn vault_enable_inbox(state: State<'_, AppState>) -> Result<String, UiError> {
    let mut guard = state.vault.lock().await;
    let open_vault = guard.as_mut().ok_or_else(UiError::no_vault)?;
    let public_key = open_vault.vault.enable_inbox().await?;
    Ok(encode_member_public_key(&public_key))
}

#[tauri::command]
pub async fn vault_disable_inbox(state: State<'_, AppState>) -> Result<(), UiError> {
    let mut guard = state.vault.lock().await;
    let open_vault = guard.as_mut().ok_or_else(UiError::no_vault)?;
    open_vault.vault.disable_inbox().await?;
    Ok(())
}

#[tauri::command]
pub async fn vault_list_inbox(state: State<'_, AppState>) -> Result<Vec<InboxListingDto>, UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    Ok(open_vault
        .vault
        .list_inbox()
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub async fn vault_collect_inbox(
    state: State<'_, AppState>,
    max_deposit_mib: u64,
    max_total_mib: u64,
) -> Result<CollectReportDto, UiError> {
    let guard = state.vault.lock().await;
    let open_vault = guard.as_ref().ok_or_else(UiError::no_vault)?;
    let limits = cu_core::CollectLimits {
        max_deposit_bytes: max_deposit_mib.saturating_mul(1024 * 1024),
        max_total_bytes: max_total_mib.saturating_mul(1024 * 1024),
    };
    Ok(open_vault.vault.collect_inbox(limits).await?.into())
}
