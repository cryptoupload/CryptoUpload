// SPDX-License-Identifier: GPL-3.0-or-later

pub mod activity;
pub mod attempts;
mod config_head;
pub mod device;
mod error;
mod find;
mod guard;
mod hardening;
pub mod inbox;
pub mod jobs;
pub mod local_state;
pub mod memlock;
#[cfg(any(target_os = "linux", windows))]
pub mod mount;
pub mod network;
pub mod overview;
mod paths;
pub mod recovery_kit;
pub mod recovery_shares;
pub mod registry;
pub mod remote;
pub mod resume;
mod share;
mod sync;
pub mod trash;
pub mod unlock;
mod unlock_methods;
mod vault;
mod vault_pin;

pub use cu_secrets::keychain;
pub use cu_secrets::member_identity;
pub use cu_sentinel as sentinel;

pub use config_head::accept_next_config_rollback;
pub use cu_crypto::seed::RecoveryPhrase;
pub use cu_crypto::x25519::StaticKeypair;
pub use cu_format::config::Credential;
pub use cu_format::config::DroppedRecipient;
pub use cu_format::config::FormatProfile;
pub use cu_format::policy::{ProgramAccess, VaultPolicy};
pub use cu_index::{RunOutcome, RunRow, VersionLocation, VersionRow};
pub use cu_jobs::MAX_RANDOM_DELAY_SECS;
pub use cu_jobs::{
    AnomalyGuard, AnomalyThresholds, AutoUnlock, HookSpec, JobManager, JobSpec, JobStatus,
    JobStore, JobsError, Outcome, PostHookOutcome, ScheduleWindow, StatusStore, TimingPrivacy,
    VaultRef, WatchHealth, window_allows,
};
pub use cu_sync::RetentionPolicy;
pub use cu_sync::hooks::HookOutcome;
pub use cu_sync::model::{Conflict, ConflictKind, ConflictResolution, Plan};
pub use cu_sync::registry_verify::{RegistryIssue, RegistryReport};
pub use cu_sync::verify::VerifyIssue;
pub use cu_sync::{CancelToken, SyncOp, SyncProgress};
pub use device::DeviceIdentity;
pub use error::CoreError;
pub use find::{DEFAULT_FIND_LIMIT, FindOutcome, FindResult, FoundEntry, name_matches};
pub use hardening::{harden_process, secure_app_data_dir};
pub use inbox::{
    CollectLimits, CollectReport, CollectedDeposit, InboxListing, SkippedDeposit,
    deposit_to_backend, deposit_via_remote,
};
pub use jobs::{
    JobUsage, SyncJobRunner, default_jobs_path, default_status_dir, job_history, job_state_db_path,
    revoke_auto_unlock, storage_usage, sync_state_db_path,
};
pub use local_state::LocalStateAccess;
#[cfg(any(target_os = "linux", windows))]
pub use mount::{
    AskFuture, ForcedUnmount, MountFlush, MountHandle, MountOptions, SentryEvents, Tripwire,
    WriteOptions,
};
pub use paths::{data_dir, mount_staging_dir, secrets_file_path};
pub use registry::{
    RegistryVerification, RegistryVerifyOptions, enable_vault_registry, vault_head_db_path,
};
pub use remote::{
    DropboxAuthorization, DropboxRemoteConfig, FileFormat, GDriveAuthorization, GDriveRemoteConfig,
    HostKeyInfo, OneDriveAuthorization, OneDriveRemoteConfig, OneDriveRootKind, RefreshTokenSink,
    RemoteConfig, S3ObjectLockMode, S3Preset, S3RemoteConfig, SecretsError, SecretsFile, SftpAuth,
    SftpRemoteConfig, WebDavRemoteConfig, begin_dropbox_oauth, begin_gdrive_oauth,
    begin_onedrive_oauth, gdrive_find_or_create_root_folder, migrate_secrets_v1,
    persist_rotated_refresh_token, s3_preset_endpoint, s3_presets, set_remote, sftp_probe_host_key,
    unlock_remote, verify_s3_exclusive_create, verify_s3_object_lock, with_rotated_refresh_token,
};
pub use resume::SqliteResumeStore;
pub use share::{OpenedShare, SHARE_EXTENSION, ShareInfo, open_share};
pub use sync::SyncJob;
pub use trash::{TrashEntry, TrashKind};
pub use unlock_methods::{FIDO2_RP_ID, UnlockMethod, set_consent_reason, set_oauth_pages};
pub use vault::{
    DetailedEntry, Entry, Fido2Auth, MemberInfo, NodeKind, ProgressEvent, RemoveMemberAuth,
    RemoveMemberOutcome, Vault,
};
pub use vault::{MIN_PASSWORD_CHARS, VaultCreation, check_new_password};
pub use vault_pin::forget_member_vault_identity;

pub use cu_auth_fido2::{Fido2Authenticator, Fido2Error, RealFido2Authenticator};
