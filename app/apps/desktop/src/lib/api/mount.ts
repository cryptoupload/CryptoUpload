import { invoke } from '@tauri-apps/api/core';

export interface MountStatus {
	mounted: boolean;
	mountpoint: string | null;
	supported: boolean;
	readOnly: boolean;
	policyReadOnly: boolean;
	pendingFiles: number;
}

export function mountStart(readOnly = false, confirmWrite = false): Promise<MountStatus> {
	return invoke('mount_start', { readOnly, confirmWrite });
}

export function mountStop(): Promise<void> {
	return invoke('mount_stop');
}

export function mountStatus(): Promise<MountStatus> {
	return invoke('mount_status');
}

export function mountOpenFolder(): Promise<void> {
	return invoke('mount_open_folder');
}
