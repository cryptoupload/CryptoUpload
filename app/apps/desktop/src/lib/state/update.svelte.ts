import { i18n } from '$lib/i18n/index.svelte';
import { networkProxyGet } from '$lib/api/security';
import { appRestart } from '$lib/api/settings';
import { check, type Update } from '@tauri-apps/plugin-updater';

export const updateState = $state<{
	pending: Update | null;
	checking: boolean;
	installing: boolean;
	error: string | null;
}>({
	pending: null,
	checking: false,
	installing: false,
	error: null,
});

export async function checkForUpdates(triggeredManually = false): Promise<void> {
	if (updateState.checking || updateState.installing) return;
	updateState.checking = true;
	updateState.error = null;
	try {
		const proxy = await networkProxyGet().catch(() => null);
		if (proxy === null || proxy.state !== 'direct') {
			if (triggeredManually) {
				updateState.error =
					i18n.t.update.pausedByProxy;
			}
			return;
		}
		const update = await check();
		updateState.pending = update;
	} catch (e) {
		if (triggeredManually) {
			updateState.error =
				i18n.t.update.checkFailed;
		}
	} finally {
		updateState.checking = false;
	}
}

export async function installUpdate(): Promise<void> {
	if (!updateState.pending || updateState.installing) return;
	updateState.installing = true;
	try {
		await updateState.pending.downloadAndInstall();
	} catch (e) {
		updateState.installing = false;
		updateState.error = i18n.t.update.installFailed;
		return;
	}
	await appRestart().catch(() => {
		updateState.installing = false;
		updateState.error = i18n.t.update.restartToFinish;
	});
}
