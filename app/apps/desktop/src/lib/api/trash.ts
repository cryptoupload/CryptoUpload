import { invoke } from '@tauri-apps/api/core';

export interface TrashEntry {
	id: string;
	originalPath: string | null;
	name: string;
	kind: 'file' | 'dir';
	deletedAt: number;
}

export function vaultTrash(vaultPath: string): Promise<void> {
	return invoke('vault_trash', { vaultPath });
}

export function trashList(): Promise<TrashEntry[]> {
	return invoke('trash_list');
}

export function trashRestore(id: string): Promise<string> {
	return invoke('trash_restore', { id });
}

export function trashPurge(id: string): Promise<void> {
	return invoke('trash_purge', { id });
}

export function trashEmpty(): Promise<void> {
	return invoke('trash_empty');
}
