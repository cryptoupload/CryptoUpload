import { invoke } from '@tauri-apps/api/core';

export type ProgramAccess = 'off' | 'allowlist' | 'ask';

export interface VaultPolicy {
	elevated: boolean;
	lockUnmountsDrive: boolean;
	lockOnSessionEvents: boolean;
	maxIdleMinutes: number;
	blockRemoteSessions: boolean;
	programAccess: ProgramAccess;
	allowedPrograms: string[];
	readOnlyDrive: boolean;
	noUnlockShortcuts: boolean;
	encryptLocalState: boolean;
	disableActivityLog: boolean;
	decoys: boolean;
	massReadThreshold: number;
	exemptPrograms: string[];
}

export type Availability = 'available' | 'unavailable' | 'planned';

export interface OptionStatus {
	id: string;
	availability: Availability;
	reason: string | null;
	caveat: string | null;
}

export interface SecurityState {
	policy: VaultPolicy;
	effective: VaultPolicy;
	options: OptionStatus[];
	twoFactor: boolean;
}

export type PolicySetOutcome =
	| { status: 'saved'; state: SecurityState; revokedJobs: string[] }
	| { status: 'passwordRequired' };

export function securityPolicyGet(): Promise<SecurityState> {
	return invoke('security_policy_get');
}

export function securityPolicySet(
	policy: VaultPolicy,
	password?: string,
	keyPin?: string
): Promise<PolicySetOutcome> {
	return invoke('security_policy_set', { policy, password: password ?? null, keyPin: keyPin ?? null });
}

export type LockReason =
	| 'idle'
	| 'screenLocked'
	| 'suspend'
	| 'sessionInactive'
	| 'remoteSession'
	| 'tripwire'
	| 'panic';

export interface ForceLockReport {
	locked: boolean;
	reason: LockReason;
	wasMounted: boolean;
	unpublished: number;
	driveStillReadable: boolean;
	cancelledJobs: number;
	detail: string | null;
}

export const FORCE_LOCKED_EVENT = 'vault-force-locked';

export interface SecurityRuntime {
	lockUnmountsDrive: boolean;
	lockOnSessionEvents: boolean;
	maxIdleMinutes: number;
}

export function securityRuntime(): Promise<SecurityRuntime> {
	return invoke('security_runtime');
}

export type AutoLockOutcome =
	| { status: 'locked'; report: ForceLockReport }
	| { status: 'skippedMounted' };

export function vaultAutoLock(): Promise<AutoLockOutcome> {
	return invoke('vault_auto_lock');
}

export function vaultForceLock(reason: LockReason): Promise<ForceLockReport> {
	return invoke('vault_force_lock', { reason });
}

export interface AppSecurity {
	hideFromCapture: boolean;
	lockMemory: boolean;
	clearRecentsOnExit: boolean;
	failedUnlockNotice: boolean;
	panicShortcut: string | null;
}

export interface MemoryLockStatus {
	locked: number;
	failed: number;
}

export function memoryLockStatus(): Promise<MemoryLockStatus | null> {
	return invoke('memory_lock_status');
}

export function appSecurityGet(): Promise<AppSecurity> {
	return invoke('app_security_get');
}

export function appSecuritySet(security: AppSecurity): Promise<AppSecurity> {
	return invoke('app_security_set', { security });
}

export interface PanicReport {
	lock: ForceLockReport;
	revokedJobs: number;
	forgottenQuickUnlocks: number;
}

export const PANIC_EVENT = 'vault-panic';

export function panicNow(): Promise<PanicReport> {
	return invoke('panic_now');
}

export interface FailedUnlock {
	at: number;
	method: string;
}

export function unlockAttemptsTake(): Promise<FailedUnlock[]> {
	return invoke('unlock_attempts_take');
}

export const ACCESS_REQUEST_EVENT = 'drive-access-request';

export interface AccessRequest {
	id: number;
	program: string;
}

export function driveAccessAnswer(id: number, allow: boolean): Promise<void> {
	return invoke('drive_access_answer', { id, allow });
}

export function driveDeniedPrograms(): Promise<string[]> {
	return invoke('drive_denied_programs');
}

export interface ProxyStatus {
	state: 'direct' | 'active' | 'broken';
	url: string | null;
	reason: string | null;
}

export function networkProxyGet(): Promise<ProxyStatus> {
	return invoke('network_proxy_get');
}

export function networkProxySet(url: string | null): Promise<ProxyStatus> {
	return invoke('network_proxy_set', { url });
}
