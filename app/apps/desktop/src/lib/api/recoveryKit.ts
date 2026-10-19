import { invoke } from '@tauri-apps/api/core';
import { i18n } from '$lib/i18n/index.svelte';

function kitLabels(): Record<string, string> {
	return { ...i18n.t.kit };
}

export function recoveryKitGenerate(): Promise<string> {
	return invoke('recovery_kit_generate', { labels: kitLabels() });
}

export function recoveryKitSave(path: string): Promise<void> {
	return invoke('recovery_kit_save', { path, labels: kitLabels() });
}
