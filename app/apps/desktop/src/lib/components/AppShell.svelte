<script lang="ts">
	import { i18n } from '$lib/i18n/index.svelte';
	import { pickPlural } from '$lib/i18n/core';
	import Rich from './Rich.svelte';
	import { onMount, tick, type Snippet } from 'svelte';
	import { session, type Section } from '$lib/state/session.svelte';
	import { checkForUpdates, updateState } from '$lib/state/update.svelte';
	import { mount, refreshMount } from '$lib/state/mount.svelte';
	import { lockVault } from '$lib/state/lock';
	import { autolock, keepUnlocked } from '$lib/state/autolock.svelte';
	import { friendlyError } from '$lib/errors';
	import { openReportBug } from '$lib/state/reportBug.svelte';

	let { children, strip }: { children: Snippet; strip?: Snippet } = $props();

	const rail = $derived<{ label: string; section: Section; icon: string }[]>([
		{ label: i18n.t.nav.vault, section: 'vault', icon: 'cu-i-home' },
		{ label: i18n.t.nav.browse, section: 'browse', icon: 'cu-i-folder' },
		{ label: i18n.t.nav.jobs, section: 'jobs', icon: 'cu-i-clock' },
		{ label: i18n.t.nav.usage, section: 'usage', icon: 'cu-i-bars' },
		{ label: i18n.t.nav.remotes, section: 'remotes', icon: 'cu-i-cloud' },
		{ label: i18n.t.nav.settings, section: 'settings', icon: 'cu-i-gear' },
	]);

	let navEl = $state<HTMLElement | null>(null);
	let indY = $state(0);
	let indH = $state(0);

	async function placeIndicator() {
		await tick();
		const active = navEl?.querySelector<HTMLElement>('[aria-current="page"]');
		if (!active) return;
		indY = active.offsetTop + 8;
		indH = active.offsetHeight - 16;
	}

	$effect(() => {
		void session.section;
		placeIndicator();
	});

	onMount(() => {
		refreshMount();
	});

	function reviewIntro() {
		session.screen = 'wizard-intro';
	}

	let nowMs = $state(Date.now());
	$effect(() => {
		if (autolock.unmountAt === null) return;
		nowMs = Date.now();
		const id = setInterval(() => (nowMs = Date.now()), 1000);
		return () => clearInterval(id);
	});
	const unmountInSeconds = $derived(
		autolock.unmountAt === null ? null : Math.max(0, Math.ceil((autolock.unmountAt - nowMs) / 1000)),
	);

	let lockBusy = $state(false);
	let lockError = $state<string | null>(null);

	async function lock() {
		lockError = null;
		lockBusy = true;
		try {
			await lockVault();
		} catch (e) {
			lockError = friendlyError(e);
		} finally {
			lockBusy = false;
		}
	}

	const lockBlockedReason = $derived(
		mount.mounted
			? i18n.t.shell.unmountBeforeLock
			: session.jobRunning
				? i18n.t.shell.waitJobBeforeLock
				: null
	);
</script>

<div class="cu-shell">
	<div class="cu-shell-main">
		<nav class="cu-rail" aria-label={i18n.t.nav.sections}>
			<div class="cu-brand">
				<svg class="cu-i-mark" width="26" height="26" viewBox="0 0 200 200" aria-hidden="true"
					><use href="#cu-i-mark" /></svg
				>
				CryptoUpload
			</div>
			<div class="cu-nav" bind:this={navEl}>
				<span
					class="cu-nav-ind"
					aria-hidden="true"
					style:transform={`translateY(${indY}px)`}
					style:height={`${indH}px`}
					style:opacity={indH > 0 ? 1 : 0}
				></span>
				{#each rail as item (item.label)}
					<button
						type="button"
						class="cu-rail-item"
						aria-current={session.section === item.section ? 'page' : undefined}
						onclick={() => (session.section = item.section)}
					>
						<svg class="cu-i"><use href="#{item.icon}" /></svg>
						<span class="cu-rail-label">{item.label}</span>
						{#if item.section === 'jobs' && session.jobRunning}
							<span class="cu-rail-dot" role="img" aria-label={i18n.t.nav.jobRunning}
							></span>
						{/if}
					</button>
				{/each}
			</div>

			<div class="cu-rail-foot">
				<button
					type="button"
					class="cu-rail-item cu-rail-minor"
					class:cu-update-available={!!updateState.pending}
					onclick={() => checkForUpdates(true)}
					disabled={updateState.checking}
				>
					{#if updateState.checking}
						{i18n.t.nav.searching}
					{:else if updateState.pending}
						<span class="cu-rail-dot" aria-hidden="true"></span>
						{i18n.t.nav.updateOne}
					{:else}
						{i18n.t.nav.updates}
					{/if}
				</button>
				<button type="button" class="cu-rail-item cu-rail-minor" onclick={reviewIntro}>
					{i18n.t.nav.help}
				</button>
				<button type="button" class="cu-rail-item cu-rail-minor" onclick={openReportBug}>
					{i18n.t.nav.report}
				</button>
				<button
					type="button"
					class="cu-rail-item cu-rail-lock"
					disabled={lockBusy || !!lockBlockedReason}
					title={lockBlockedReason ?? undefined}
					onclick={lock}
				>
					<svg class="cu-i"><use href="#cu-i-lock" /></svg>
					<span class="cu-rail-label">{i18n.t.nav.lockVault}</span>
				</button>
				<div class="cu-rail-note-slot">
					{#if lockBlockedReason}
						<p
							class="cu-rail-note"
						>
							{lockBlockedReason}
						</p>
					{/if}
					{#if lockError}
						<p class="cu-rail-note cu-rail-error" role="alert">{lockError}</p>
					{/if}
				</div>
			</div>
		</nav>
		<main class="cu-stage">
			{#if unmountInSeconds !== null}
				<div class="cu-lock-warning" role="alert">
					<span>
						<Rich text={pickPlural(i18n.language.tag, unmountInSeconds, i18n.t.shell.autoLockWarning)} values={{ n: unmountInSeconds }} strongClass="cu-num" />
					</span>
					<button type="button" class="cu-btn sm" onclick={keepUnlocked}>
						{i18n.t.shell.stayUnlocked}
					</button>
				</div>
			{/if}
			{#key session.section}
				<div class="cu-view">
					{@render children()}
				</div>
			{/key}
		</main>
	</div>
	{#if strip}
		{@render strip()}
	{/if}
</div>

<style>
	.cu-lock-warning {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
		margin: 16px 24px 0;
		padding: 10px 14px;
		border: 1px solid var(--cu-warn);
		border-radius: var(--cu-radius-sm);
		font-size: 13px;
	}

	.cu-shell {
		position: relative;
		height: 100%;
		display: grid;
		grid-template-rows: minmax(0, 1fr) auto;
		isolation: isolate;
	}

	.cu-shell::before,
	.cu-shell::after {
		content: '';
		position: absolute;
		inset: 0;
		pointer-events: none;
		z-index: 40;
	}

	.cu-shell::before {
		background: var(--cu-grain);
		opacity: var(--cu-grain-opacity);
	}

	.cu-shell::after {
		z-index: 39;
		background: radial-gradient(ellipse at 50% 40%, transparent 55%, var(--cu-vignette) 100%);
	}

	.cu-shell-main {
		display: grid;
		grid-template-columns: 208px minmax(0, 1fr);
		min-height: 0;
	}

	.cu-rail {
		background: var(--cu-rail);
		border-inline-end: 1px solid var(--cu-line);
		padding: 20px 12px 14px;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow-y: auto;
	}

	.cu-brand {
		display: flex;
		align-items: center;
		gap: 10px;
		font: 400 22px/1 var(--cu-font-display);
		letter-spacing: 0.01em;
		padding: 2px 10px 22px;
		color: var(--cu-text);
	}

	.cu-brand .cu-i-mark {
		color: var(--cu-seal-text);
		flex-shrink: 0;
	}

	.cu-nav {
		position: relative;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.cu-nav-ind {
		position: absolute;
		inset-inline-start: -12px;
		top: 0;
		width: 3px;
		border-radius: 0 3px 3px 0;
		background: var(--cu-seal-text);
		transition:
			transform 220ms var(--cu-ease-out),
			height 200ms,
			opacity 200ms;
	}

	.cu-rail-item {
		display: flex;
		align-items: center;
		gap: 11px;
		width: 100%;
		background: none;
		border: 0;
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		color: var(--cu-muted);
		font-size: 14px;
		text-align: start;
		cursor: pointer;
		transition:
			background var(--cu-motion-fast),
			color var(--cu-motion-fast);
	}

	.cu-rail-item:hover:not(:disabled),
	.cu-rail-item[aria-current='page'] {
		color: var(--cu-text);
		background: var(--cu-surface);
	}

	.cu-rail-item:disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}

	.cu-rail-label {
		flex: 1;
		min-width: 0;
	}

	.cu-rail-dot {
		width: 6px;
		height: 6px;
		flex-shrink: 0;
		border-radius: 50%;
		background: var(--cu-seal-text);
		animation: cu-rail-pulse 1.6s ease-in-out infinite;
	}

	@keyframes cu-rail-pulse {
		0%,
		100% {
			opacity: 0.5;
		}
		50% {
			opacity: 1;
		}
	}

	.cu-rail-foot {
		margin-top: auto;
		padding-top: 16px;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.cu-rail-minor {
		font-size: 13px;
		padding: 6px 10px;
	}

	.cu-rail-minor.cu-update-available {
		color: var(--cu-seal-text);
		font-weight: 500;
	}

	.cu-rail-lock {
		margin-top: 8px;
		padding-top: 10px;
		border-top: 1px solid var(--cu-line);
		border-radius: 0 0 var(--cu-radius-sm) var(--cu-radius-sm);
	}

	.cu-rail-note-slot {
		min-height: 72px;
	}

	.cu-rail-note {
		margin: 4px 10px 0;
		font-size: 12px;
		line-height: 1.4;
		color: var(--cu-muted);
	}

	.cu-rail-error {
		color: var(--cu-conflict);
	}

	.cu-stage {
		position: relative;
		min-width: 0;
		min-height: 0;
		overflow: auto;
		background: radial-gradient(70% 50% at 12% -8%, var(--cu-key-light), transparent 70%);
		background-attachment: local;
	}

	.cu-stage::-webkit-scrollbar {
		width: 8px;
	}

	.cu-stage::-webkit-scrollbar-thumb {
		background: var(--cu-line-strong);
		border-radius: 4px;
	}

	.cu-view {
		min-height: 100%;
		animation: cu-view-in var(--cu-motion-fast) ease-out;
	}

	@keyframes cu-view-in {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}
</style>
