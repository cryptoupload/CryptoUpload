import {
	isPermissionGranted,
	requestPermission,
	sendNotification,
} from '@tauri-apps/plugin-notification';
import { notificationsEnabled } from '$lib/state/notifications.svelte';

let permissionRequestedThisSession = false;

export async function notify(title: string, body: string): Promise<void> {
	try {
		if (!notificationsEnabled()) return;

		let granted = await isPermissionGranted();
		if (!granted && !permissionRequestedThisSession) {
			permissionRequestedThisSession = true;
			const permission = await requestPermission();
			granted = permission === 'granted';
		}
		if (!granted) return;

		sendNotification({ title, body });
	} catch {
	}
}
