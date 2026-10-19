import { invoke } from '@tauri-apps/api/core';
import { i18n } from '$lib/i18n/index.svelte';
import { noteRemoteRefusal, remoteConsent, resetRemoteConsent } from '$lib/state/remoteConsent.svelte';

async function unlockCall(command: string, args: Record<string, unknown>): Promise<VaultInfo> {
	try {
		const info = await invoke<VaultInfo>(command, { ...args, allowRemoteSession: remoteConsent.allow });
		resetRemoteConsent();
		return info;
	} catch (e) {
		noteRemoteRefusal(e);
		throw e;
	}
}

export interface VaultInfo {
	fileFormat: string;
	nameFormat: string;
	kdfName: string;
	seedCount: number;
	hasRecoveryPhrase: boolean;
	privateProfile: boolean;
	decoyFiles: boolean;
}

export function vaultCreate(
	root: string,
	password: string,
	privateProfile = false,
	decoyFiles = false
): Promise<VaultInfo> {
	return invoke('vault_create', {
		root,
		password,
		privateProfile,
		decoyFiles,
		readmeText: i18n.t.outside.readme
	});
}

export function vaultUnlock(root: string, password: string, keyPin?: string): Promise<VaultInfo> {
	return unlockCall('vault_unlock', { root, password, keyPin: keyPin ?? null });
}

export function vaultUnlockRecovery(root: string, words: string): Promise<VaultInfo> {
	return unlockCall('vault_unlock_recovery', { root, words });
}

export function vaultCreateRemote(
	remoteName: string,
	password: string,
	privateProfile = false,
	decoyFiles = false
): Promise<VaultInfo> {
	return invoke('vault_create_remote', {
		remoteName,
		password,
		privateProfile,
		decoyFiles,
		readmeText: i18n.t.outside.readme
	});
}

export function vaultUnlockRemote(
	remoteName: string,
	password: string,
	keyPin?: string
): Promise<VaultInfo> {
	return unlockCall('vault_unlock_remote', { remoteName, password, keyPin: keyPin ?? null });
}

export function vaultGenerateRecoveryPhrase(replace: boolean): Promise<string[]> {
	return invoke('vault_generate_recovery_phrase', { replace });
}

export function vaultChangePassword(newPassword: string): Promise<void> {
	return invoke('vault_change_password', { newPassword });
}

export function vaultLock(): Promise<void> {
	return invoke('vault_lock');
}

export interface LockNotice {
	forcedLock: import('./security').ForceLockReport | null;
	autoLocked: boolean;
}

export function webviewRecycle(notice: LockNotice): Promise<boolean> {
	return invoke('webview_recycle', { notice });
}

export function lockNoticeTake(): Promise<LockNotice | null> {
	return invoke('lock_notice_take');
}

export type UnlockMethod =
	| { kind: 'password' }
	| { kind: 'recoverySeed' }
	| { kind: 'fido2'; rpId: string }
	| { kind: 'platform' }
	| { kind: 'twoFactor' }
	| { kind: 'member'; name: string; publicKey: string }
	| { kind: 'unknown'; kid: string };

export function hasUnlockMethod(methods: UnlockMethod[], kind: UnlockMethod['kind']): boolean {
	return methods.some((m) => m.kind === kind);
}

export function vaultPeekUnlockMethods(root: string): Promise<UnlockMethod[]> {
	return invoke('vault_peek_unlock_methods', { root });
}

export function vaultUnlockMethods(): Promise<UnlockMethod[]> {
	return invoke('vault_unlock_methods');
}

export function vaultUnlockPlatform(root: string): Promise<VaultInfo> {
	return unlockCall('vault_unlock_platform', { root });
}

export function vaultUnlockFido2(root: string, pin?: string): Promise<VaultInfo> {
	return unlockCall('vault_unlock_fido2', { root, pin: pin ?? null });
}

export function recoverySplit(words: string, threshold: number, count: number): Promise<string[]> {
	return invoke('recovery_split', { words, threshold, count });
}

export function vaultEnableTwoFactor(password: string, pin?: string): Promise<void> {
	return invoke('vault_enable_two_factor', { password, pin: pin ?? null });
}

export function vaultDisableTwoFactor(newPassword: string): Promise<void> {
	return invoke('vault_disable_two_factor', { newPassword });
}

export function vaultChangePasswordTwoFactor(
	currentPassword: string,
	newPassword: string,
	pin?: string
): Promise<void> {
	return invoke('vault_change_password_two_factor', { currentPassword, newPassword, pin: pin ?? null });
}

export function vaultEnablePlatformUnlock(): Promise<void> {
	return invoke('vault_enable_platform_unlock');
}

export function vaultDisablePlatformUnlock(): Promise<void> {
	return invoke('vault_disable_platform_unlock');
}

export function vaultEnableFido2(pin?: string): Promise<void> {
	return invoke('vault_enable_fido2', { pin: pin ?? null });
}

export function vaultDisableFido2(): Promise<void> {
	return invoke('vault_disable_fido2');
}

export function memberIdentityExists(): Promise<boolean> {
	return invoke('member_identity_exists');
}

export function memberIdentityGenerate(passphrase: string): Promise<string> {
	return invoke('member_identity_generate', { passphrase });
}

export function memberIdentityPublicKey(passphrase: string): Promise<string> {
	return invoke('member_identity_public_key', { passphrase });
}

export function vaultAcceptConfigRollback(): Promise<void> {
	return invoke('vault_accept_config_rollback');
}

export function vaultUnlockMember(
	root: string,
	identityPassphrase: string,
	acceptNewVault = false
): Promise<VaultInfo> {
	return unlockCall('vault_unlock_member', { root, identityPassphrase, acceptNewVault });
}

export interface MemberInfo {
	name: string;
	publicKey: string;
}

export function vaultListMembers(): Promise<MemberInfo[]> {
	return invoke('vault_list_members');
}

export function vaultAddMember(name: string, publicKeyB64: string): Promise<void> {
	return invoke('vault_add_member', { name, publicKeyB64 });
}

export interface RemoveMemberResult {
	newRecoveryPhrase: string[] | null;
	dropped: ('fido2' | 'platform' | 'unknown')[];
}

export function vaultRemoveMember(
	name: string,
	password: string,
	useFido2: boolean,
	pin?: string
): Promise<RemoveMemberResult> {
	return invoke('vault_remove_member', { name, password, useFido2, pin: pin || null });
}

export interface InboxListing {
	depositName: string;
	encryptedSize: number | null;
}

export interface CollectedDeposit {
	vaultPath: string;
	originalName: string;
	depositorLabel: string;
	bytes: number;
}

export interface SkippedDeposit {
	depositName: string;
	reason: string;
}

export interface CollectReport {
	collected: CollectedDeposit[];
	skipped: SkippedDeposit[];
}

export function vaultInboxStatus(): Promise<[boolean, string | null]> {
	return invoke('vault_inbox_status');
}

export function vaultEnableInbox(): Promise<string> {
	return invoke('vault_enable_inbox');
}

export function vaultDisableInbox(): Promise<void> {
	return invoke('vault_disable_inbox');
}

export function vaultListInbox(): Promise<InboxListing[]> {
	return invoke('vault_list_inbox');
}

export function vaultCollectInbox(
	maxDepositMib: number,
	maxTotalMib: number
): Promise<CollectReport> {
	return invoke('vault_collect_inbox', { maxDepositMib, maxTotalMib });
}
