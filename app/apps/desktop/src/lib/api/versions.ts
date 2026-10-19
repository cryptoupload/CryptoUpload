import { invoke } from '@tauri-apps/api/core';

export interface Version {
	version: number;
	createdAt: number;
	size: number;
	name: string;
	deleted: boolean;
	location: 'canonical' | 'archived';
}

export interface VaultVersionsResult {
	noJobConfigured: boolean;
	versions: Version[];
}

export function vaultVersions(path: string): Promise<VaultVersionsResult> {
	return invoke('vault_versions', { path });
}

export function vaultRestoreVersion(path: string, version: number, dest: string): Promise<void> {
	return invoke('vault_restore_version', { path, version, dest });
}

export interface VerifyIssue {
	vaultPath: string;
	problem: string;
}

export interface JobVerifyResult {
	level: number;
	issues: VerifyIssue[];
}

export function jobVerify(name: string, level: 1 | 2): Promise<JobVerifyResult> {
	return invoke('job_verify', { name, level });
}
