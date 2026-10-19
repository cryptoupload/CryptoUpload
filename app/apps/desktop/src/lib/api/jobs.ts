import { invoke, Channel } from '@tauri-apps/api/core';

export interface ScheduleWindow {
	days: number;
	fromMin: number;
	toMin: number;
}

export interface HookSpec {
	program: string;
	args: string[];
	timeoutSecs: number;
}

export interface Job {
	name: string;
	enabled: boolean;
	localRoot: string;
	vaultPath: string;
	intervalSecs: number;
	keepVersions: number;
	keepRuns: number;
	autoUnlock: boolean;
	matchesOpenVault: boolean;
	excludes: string[];
	scheduleWindows: ScheduleWindow[];
	preHook: HookSpec | null;
	postHook: HookSpec | null;
	anomalyGuard: AnomalyGuard;
	timing: JobTiming;
}

export interface JobTiming {
	shuffleUploads: boolean;
	randomDelayMaxSecs: number;
}

export interface AnomalyGuard {
	enabled: boolean;
	massModifyAbs: number;
	massModifyMultiplier: number;
	massDeleteAbs: number;
	massDeleteMultiplier: number;
	renameMinCount: number;
	renameUniformFractionPct: number;
	minBaselineRuns: number;
}

export type JobProgressEvent =
	| { kind: 'item'; vaultPath: string; op: string; index: number; total: number }
	| { kind: 'bytes'; count: number }
	| { kind: 'plan'; items: number; uploadBytes: number };

export interface ConflictInfo {
	vaultPath: string;
	detail: string;
}

export type PostHookOutcome =
	| { kind: 'ok' }
	| { kind: 'failed'; exitCode: number | null; stderrTail: string }
	| { kind: 'timedOut' }
	| { kind: 'spawnFailed'; reason: string };

export interface RunSummary {
	uploads: number;
	deletes: number;
	renames: number;
	bytesTransferred: number;
	conflicts: ConflictInfo[];
	ignoredKept: number;
	postHook: PostHookOutcome | null;
}

export type JobRunNowOutcome =
	| ({ kind: 'completed' } & RunSummary)
	| {
			kind: 'anomalyBlocked';
			reason: string;
			uploadsAffected: number;
			deletesAffected: number;
			renamesAffected: number;
	  }
	| { kind: 'cancelled' };

export interface RunRow {
	runId: number;
	startedAt: number;
	finishedAt: number | null;
	outcome: 'ok' | 'failed' | 'interrupted' | null;
	filesUploaded: number;
	filesDeleted: number;
	filesRenamed: number;
	bytesTransferred: number;
}

export function jobList(): Promise<Job[]> {
	return invoke('job_list');
}

export function jobAdd(
	name: string,
	localRoot: string,
	vaultPath: string,
	intervalSecs: number,
	keepVersions: number,
	keepRuns: number,
): Promise<void> {
	return invoke('job_add', { name, localRoot, vaultPath, intervalSecs, keepVersions, keepRuns });
}

export function jobRemove(name: string): Promise<boolean> {
	return invoke('job_remove', { name });
}

export function jobSetEnabled(name: string, enabled: boolean): Promise<boolean> {
	return invoke('job_set_enabled', { name, enabled });
}

export function jobEnableAutoUnlock(name: string, password: string): Promise<void> {
	return invoke('job_enable_auto_unlock', { name, password });
}

export function jobHistory(name: string, limit: number): Promise<RunRow[]> {
	return invoke('job_history', { name, limit });
}

export function jobRunNow(
	name: string,
	onProgress: (event: JobProgressEvent) => void,
	confirmAnomaly = false,
): Promise<JobRunNowOutcome> {
	const channel = new Channel<JobProgressEvent>();
	channel.onmessage = onProgress;
	return invoke('job_run_now', { name, onProgress: channel, confirmAnomaly });
}

export function jobTopLevelFolders(name: string): Promise<string[]> {
	return invoke('job_top_level_folders', { name });
}

export function jobSetExcludes(name: string, excludes: string[]): Promise<boolean> {
	return invoke('job_set_excludes', { name, excludes });
}

export function jobSetScheduleWindows(
	name: string,
	scheduleWindows: ScheduleWindow[],
): Promise<boolean> {
	return invoke('job_set_schedule_windows', { name, scheduleWindows });
}

export function jobSetPreHook(name: string, preHook: HookSpec | null): Promise<boolean> {
	return invoke('job_set_pre_hook', { name, preHook });
}

export function jobSetPostHook(name: string, postHook: HookSpec | null): Promise<boolean> {
	return invoke('job_set_post_hook', { name, postHook });
}

export function jobSetTiming(name: string, timing: JobTiming): Promise<boolean> {
	return invoke('job_set_timing', { name, timing });
}

export function jobSetAnomalyGuard(name: string, anomalyGuard: AnomalyGuard): Promise<boolean> {
	return invoke('job_set_anomaly_guard', { name, anomalyGuard });
}
