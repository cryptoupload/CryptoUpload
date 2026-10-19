import { invoke } from '@tauri-apps/api/core';

export type Theme = 'system' | 'light' | 'dark';

export type Motion = 'full' | 'reduced' | 'off';

export interface UiSettings {
	theme: Theme;
	onboarded: boolean;
	introSeen: boolean;
	recentVaults: string[];
	lastMountpoint: string | null;
	autoLockMinutes: number;
	reopenLastVault: boolean;
	autoUpdateCheck: boolean;
	notificationsEnabled: boolean;
	bandwidthLimitKib: number;
	motion: Motion;
	language: string | null;
	runInBackground: boolean;
	startAtLogin: boolean;
}

export function uiSettingsGet(): Promise<UiSettings> {
	return invoke('ui_settings_get');
}

export function firstLocalVault(recentVaults: string[]): string | null {
	return recentVaults.find((v) => !v.startsWith('remote:')) ?? null;
}

export function uiSettingsSet(patch: {
	theme?: Theme;
	onboarded?: boolean;
	introSeen?: boolean;
	autoLockMinutes?: number;
	reopenLastVault?: boolean;
	autoUpdateCheck?: boolean;
	notificationsEnabled?: boolean;
	bandwidthLimitKib?: number;
	motion?: Motion;
	language?: string;
	runInBackground?: boolean;
	startAtLogin?: boolean;
}): Promise<UiSettings> {
	return invoke('ui_settings_set', patch);
}

export function translationExport(path: string, content: string): Promise<void> {
	return invoke('translation_export', { path, content });
}

export interface BugReportInfo {
	version: string;
	os: string;
	arch: string;
}

export function bugReportInfo(): Promise<BugReportInfo> {
	return invoke('bug_report_info');
}

export function appRestart(): Promise<void> {
	return invoke('app_restart');
}

export async function setOutsideTexts(
	consentReason: string,
	oauthGranted: string,
	oauthDenied: string,
	trayOpen: string,
	trayQuit: string,
	backgroundNotice: string
): Promise<void> {
	const { i18n } = await import('$lib/i18n/index.svelte');
	return invoke('set_outside_texts', {
		consentReason,
		oauthGranted,
		oauthDenied,
		trayOpen,
		trayQuit,
		backgroundNotice,
		sleepReason: i18n.t.outside.sleepReason,
	});
}
