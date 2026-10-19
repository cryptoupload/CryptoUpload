import { invoke, Channel } from '@tauri-apps/api/core';

export interface Entry {
	name: string;
	kind: 'file' | 'dir';
	size: number | null;
	modified: number | null;
}

export interface DetailedEntry {
	name: string;
	encryptedName: string;
	kind: 'file' | 'dir';
}

export interface RevealedListing {
	physicalDir: string;
	entries: DetailedEntry[];
}

export type TransferEvent =
	| { kind: 'file'; vaultPath: string }
	| { kind: 'bytes'; count: number }
	| { kind: 'item'; index: number };

export function vaultList(vaultDir: string): Promise<Entry[]> {
	return invoke('vault_list', { vaultDir });
}

export interface FoundEntry {
	path: string;
	isDir: boolean;
}

export interface FindOutcome {
	truncated: boolean;
	cancelled: boolean;
}

export function vaultFindStream(
	startDir: string,
	pattern: string,
	onResult: (entry: FoundEntry) => void,
): Promise<FindOutcome> {
	const channel = new Channel<FoundEntry>();
	channel.onmessage = onResult;
	return invoke('vault_find_stream', { startDir, pattern, onResult: channel });
}

export function vaultFindCancel(): Promise<void> {
	return invoke('vault_find_cancel');
}

export function vaultListRevealed(vaultDir: string): Promise<RevealedListing> {
	return invoke('vault_list_revealed', { vaultDir });
}

export function vaultMkdir(vaultPath: string): Promise<void> {
	return invoke('vault_mkdir', { vaultPath });
}

export function localPathKinds(paths: string[]): Promise<boolean[]> {
	return invoke('local_path_kinds', { paths });
}

export interface ImportItem {
	localSrc: string;
	vaultDest: string;
}

export function vaultImportMany(
	items: ImportItem[],
	onProgress: (event: TransferEvent) => void,
): Promise<(string | null)[]> {
	const channel = new Channel<TransferEvent>();
	channel.onmessage = onProgress;
	return invoke('vault_import_many', { items, onProgress: channel });
}

export function vaultExport(
	vaultSrc: string,
	localDest: string,
	onProgress: (event: TransferEvent) => void,
): Promise<void> {
	const channel = new Channel<TransferEvent>();
	channel.onmessage = onProgress;
	return invoke('vault_export', { vaultSrc, localDest, onProgress: channel });
}
