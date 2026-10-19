import { invoke } from '@tauri-apps/api/core';

export function inboxDeposit(
	remoteName: string,
	pubkeyB64: string,
	filePath: string,
	label: string
): Promise<void> {
	return invoke('inbox_deposit', { remoteName, pubkeyB64, filePath, label });
}
