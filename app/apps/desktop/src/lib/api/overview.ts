import { invoke } from '@tauri-apps/api/core';

export type ActivityKind =
	| { kind: 'runFinished'; job: string; uploaded: number; deleted: number; renamed: number; bytes: number; conflicts: number }
	| { kind: 'runFailed'; job: string; message: string }
	| { kind: 'runCancelled'; job: string }
	| { kind: 'conflictKeptBoth'; job: string; path: string; detail: string }
	| { kind: 'anomalyBlocked'; job: string; reason: string }
	| { kind: 'verifyPassed'; job: string; files: number }
	| { kind: 'verifyIssues'; job: string; issues: number }
	| { kind: 'imported'; path: string; files: number }
	| { kind: 'trashEmptied'; items: number };

export type ActivityRecord = ActivityKind & { at: number; warning: boolean };

export interface RecentFile {
	job: string;
	path: string;
	size: number;
	modified: number;
}

export type EntrySyncState = 'synced' | 'pending' | 'conflict';

export interface ServiceJobStatus {
	job: string;
	serviceActive: boolean;
	nextRun: number | null;
	watcherHealth: string;
}

export function activityRecent(vaultRoot: string, limit: number): Promise<ActivityRecord[]> {
	return invoke('activity_recent', { vaultRoot, limit });
}

export function vaultRecent(vaultRoot: string, limit: number): Promise<RecentFile[]> {
	return invoke('vault_recent', { vaultRoot, limit });
}

export function vaultSyncStates(
	vaultRoot: string,
	dir: string,
	names: string[],
): Promise<Record<string, EntrySyncState>> {
	return invoke('vault_sync_states', { vaultRoot, dir, names });
}

export function jobsServiceStatus(vaultRoot: string): Promise<ServiceJobStatus[]> {
	return invoke('jobs_service_status', { vaultRoot });
}

export function jobCancel(name: string): Promise<boolean> {
	return invoke('job_cancel', { name });
}
