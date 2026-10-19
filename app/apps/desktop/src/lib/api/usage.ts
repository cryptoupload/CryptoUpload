import { invoke } from '@tauri-apps/api/core';

export interface JobUsage {
	jobName: string;
	fileCount: number;
	totalBytes: number;
	versionCount: number;
	versionBytes: number;
	totalBytesTransferredLifetime: number;
}

export function storageUsage(): Promise<JobUsage[]> {
	return invoke('storage_usage');
}
