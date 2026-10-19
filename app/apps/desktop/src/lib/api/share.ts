import { invoke, Channel } from '@tauri-apps/api/core';
import type { TransferEvent } from '$lib/api/browse';

export interface ShareInfo {
	originalName: string;
	bytes: number;
}

export interface OpenedShare {
	path: string;
	originalName: string;
	bytes: number;
}

export function vaultShareFile(
	vaultSrc: string,
	out: string,
	password: string,
	overwrite: boolean,
	onProgress: (event: TransferEvent) => void,
): Promise<ShareInfo> {
	const channel = new Channel<TransferEvent>();
	channel.onmessage = onProgress;
	return invoke('vault_share_file', { vaultSrc, out, password, overwrite, onProgress: channel });
}

export function openSharePackage(
	pkg: string,
	password: string,
	destDir: string,
	onProgress: (event: TransferEvent) => void,
): Promise<OpenedShare> {
	const channel = new Channel<TransferEvent>();
	channel.onmessage = onProgress;
	return invoke('open_share_package', { package: pkg, password, destDir, onProgress: channel });
}
