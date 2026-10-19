import { invoke } from '@tauri-apps/api/core';

export interface RegistryIssue {
	path: string;
	problem: string;
}

export interface RegistryVerifyResult {
	filesOk: number;
	dirsOk: number;
	unregistered: number;
	mismatches: RegistryIssue[];
	missing: RegistryIssue[];
	unresolvedPending: number;
	registered: number;
	sealed: boolean;
	clean: boolean;
}

export function vaultRegistryVerify(
	register: boolean,
	seal: boolean,
): Promise<RegistryVerifyResult> {
	return invoke('vault_registry_verify', { register, seal });
}

export function vaultRegistryReset(): Promise<RegistryVerifyResult> {
	return invoke('vault_registry_reset');
}
