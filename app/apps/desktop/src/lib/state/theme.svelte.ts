import { uiSettingsGet, uiSettingsSet, type Motion, type Theme } from '$lib/api/settings';

export const theme = $state<{ current: Theme; motion: Motion }>({
	current: 'system',
	motion: 'full',
});

export function sceneMotionOff(): boolean {
	if (theme.motion !== 'full') return true;
	return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

function applyMotion(m: Motion) {
	document.documentElement.dataset.motion = m;
}

export function resolveThemeAttribute(t: Theme): 'light' | 'dark' | undefined {
	if (t === 'light') return 'light';
	if (t === 'dark') return 'dark';
	return undefined;
}

function applyToDocument(t: Theme) {
	const attr = resolveThemeAttribute(t);
	if (attr) {
		document.documentElement.dataset.theme = attr;
	} else {
		delete document.documentElement.dataset.theme;
	}
}

export async function loadTheme(): Promise<void> {
	const settings = await uiSettingsGet();
	theme.current = settings.theme;
	theme.motion = settings.motion ?? 'full';
	applyToDocument(theme.current);
	applyMotion(theme.motion);
}

export async function setMotion(m: Motion): Promise<void> {
	theme.motion = m;
	applyMotion(m);
	await uiSettingsSet({ motion: m });
}

export async function setTheme(t: Theme): Promise<void> {
	theme.current = t;
	applyToDocument(t);
	await uiSettingsSet({ theme: t });
}
