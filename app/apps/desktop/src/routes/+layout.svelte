<script lang="ts">
	import { onMount } from 'svelte';
	import '../app.css';
	import { loadTheme } from '$lib/state/theme.svelte';
	import { i18n, loadLanguage } from '$lib/i18n/index.svelte';
	import { uiSettingsGet } from '$lib/api/settings';
	import { session } from '$lib/state/session.svelte';
	import { checkForUpdates } from '$lib/state/update.svelte';
	import { initAutoLock } from '$lib/state/autolock.svelte';
	import { listen } from '@tauri-apps/api/event';
	import { FORCE_LOCKED_EVENT, PANIC_EVENT, type ForceLockReport, type PanicReport } from '$lib/api/security';
	import { applyForcedLock, restoreLockNotice } from '$lib/state/lock';
	import { initNotifications } from '$lib/state/notifications.svelte';
	import UpdateBanner from '$lib/components/UpdateBanner.svelte';
	import DriveAccessPrompt from '$lib/components/DriveAccessPrompt.svelte';
	import RecoveryKitPanel from '$lib/components/RecoveryKitPanel.svelte';
	import TranslateHelp from '$lib/components/TranslateHelp.svelte';
	import ReportBug from '$lib/components/ReportBug.svelte';
	import IntroScene from '$lib/components/IntroScene.svelte';
	import { intro, showIntroScene } from '$lib/state/intro.svelte';
	import { setOutsideTexts } from '$lib/api/settings';

	let { children } = $props();

	$effect(() => {
		const t = i18n.t.outside;
		void setOutsideTexts(t.consentReason, t.oauthGranted, t.oauthDenied, t.trayOpen, t.trayQuit, t.backgroundNotice);
	});

	onMount(async () => {
		loadTheme();
		void listen<ForceLockReport>(FORCE_LOCKED_EVENT, (event) => applyForcedLock(event.payload));
		void listen<PanicReport>(PANIC_EVENT, (event) => applyForcedLock(event.payload.lock));
		await restoreLockNotice();
		await loadLanguage();
		try {
			const settings = await uiSettingsGet();
			if (!settings.introSeen) showIntroScene();
			session.screen = settings.onboarded ? 'unlock' : 'wizard-intro';
			initAutoLock(settings.autoLockMinutes);
			initNotifications(settings.notificationsEnabled);
			if (settings.autoUpdateCheck) setTimeout(checkForUpdates, 3000);
		} catch {
			session.screen = 'wizard-intro';
			initAutoLock(15);
			initNotifications(true);
			setTimeout(checkForUpdates, 3000);
		}
	});
</script>

<div class="cu-app-root" inert={intro.visible}>
	{@render children()}
	<UpdateBanner />
	<DriveAccessPrompt />
	<RecoveryKitPanel />
	<TranslateHelp />
	<ReportBug />
</div>
{#if intro.visible}
	<IntroScene />
{/if}

<style>
	.cu-app-root {
		display: contents;
	}
</style>
