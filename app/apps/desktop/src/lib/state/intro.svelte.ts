import { uiSettingsSet } from '$lib/api/settings';

export const intro = $state<{ visible: boolean }>({ visible: false });

export function showIntroScene() {
	intro.visible = true;
}

export async function finishIntroScene() {
	intro.visible = false;
	try {
		await uiSettingsSet({ introSeen: true });
	} catch {
	}
}
