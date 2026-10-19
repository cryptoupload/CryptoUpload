<script lang="ts">
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import { open } from '@tauri-apps/plugin-dialog';
	import { downloadDir } from '@tauri-apps/api/path';
	import { session } from '$lib/state/session.svelte';
	import { friendlyError } from '$lib/errors';
	import { formatBytes } from '$lib/format';
	import { openSharePackage, type OpenedShare } from '$lib/api/share';

	let packagePath = $state('');
	let password = $state('');
	let destDir = $state('');
	let busy = $state(false);
	let progress = $state(0);
	let error = $state<string | null>(null);
	let opened = $state<OpenedShare | null>(null);

	$effect(() => {
		if (destDir) return;
		downloadDir()
			.then((dir) => {
				if (!destDir) destDir = dir;
			})
			.catch(() => {});
	});

	async function pickPackage() {
		const picked = await open({
			directory: false,
			multiple: false,
			filters: [{ name: i18n.t.outside.sharePackage, extensions: ['cushare'] }],
		});
		if (typeof picked === 'string') packagePath = picked;
	}

	async function pickDest() {
		const picked = await open({ directory: true, multiple: false });
		if (typeof picked === 'string') destDir = picked;
	}

	async function openPackage(e: Event) {
		e.preventDefault();
		error = null;
		opened = null;
		if (!packagePath.trim()) {
			error = i18n.t.openShare.chooseFile;
			return;
		}
		if (!password) {
			error = i18n.t.openShare.passwordNeeded;
			return;
		}
		if (!destDir.trim()) {
			error = i18n.t.openShare.chooseDest;
			return;
		}
		busy = true;
		progress = 0;
		try {
			opened = await openSharePackage(packagePath.trim(), password, destDir.trim(), (event) => {
				if (event.kind === 'bytes') progress = event.count;
			});
			password = '';
		} catch (e) {
			error = friendlyError(e);
		} finally {
			busy = false;
		}
	}

	function back() {
		session.screen = session.screenBeforeOpenShare;
	}
</script>

<div class="cu-gate">
	<div class="cu-open-share">
		<button type="button" class="cu-back cu-btn ghost sm" onclick={back}>
			{i18n.t.common.back}
		</button>

		<h1>{i18n.t.openShare.title}</h1>
		<p class="cu-subtitle">
			{i18n.t.openShare.intro}
		</p>

		<form onsubmit={openPackage}>
			<label class="cu-field">
				<span>{i18n.t.openShare.fileLabel}</span>
				<span class="cu-path-row">
					<input type="text" bind:value={packagePath} placeholder={i18n.t.openShare.filePlaceholder} />
					<button type="button" class="cu-btn" onclick={pickPackage}>{i18n.t.common.choose}</button>
				</span>
			</label>
			<label class="cu-field">
				<span>{i18n.t.share.packagePassword}</span>
				<input type="password" bind:value={password} autocomplete="off" />
			</label>
			<label class="cu-field">
				<span>{i18n.t.openShare.destLabel}</span>
				<span class="cu-path-row">
					<input type="text" bind:value={destDir} placeholder={i18n.t.openShare.destPlaceholder} />
					<button type="button" class="cu-btn" onclick={pickDest}>{i18n.t.common.choose}</button>
				</span>
			</label>
			<button type="submit" class="cu-primary" disabled={busy}>
				{busy ? fill(i18n.t.openShare.opening, { done: formatBytes(progress) }) : i18n.t.openShare.submit}
			</button>
		</form>

		{#if opened}
			<p class="cu-success" role="status">
				{fill(i18n.t.openShare.opened, { name: opened.originalName, size: formatBytes(opened.bytes) })}
				<span class="cu-mono">{opened.path}</span>
			</p>
		{/if}
		{#if error}
			<p class="cu-error" role="alert">{error}</p>
		{/if}
	</div>
</div>

<style>
	.cu-gate {
		min-height: 100%;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 32px;
	}

	.cu-open-share {
		width: 100%;
		max-width: 480px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		box-shadow: var(--cu-shadow);
		padding: 24px 28px 28px;
	}

	.cu-back {
		margin: 0 0 12px -12px;
	}

	h1 {
		margin: 0 0 8px;
		font-family: var(--cu-font-display);
		font-weight: 400;
		font-size: 26px;
		letter-spacing: -0.005em;
	}

	.cu-subtitle {
		color: var(--cu-muted);
		font-size: 13px;
		margin: 0 0 24px;
	}

	.cu-field {
		display: flex;
		flex-direction: column;
		gap: 6px;
		font-size: 13px;
		margin-bottom: 14px;
	}

	.cu-field input {
		height: 34px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		color: var(--cu-text);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		font-size: 13px;
		min-width: 0;
	}

	.cu-path-row {
		display: flex;
		gap: 8px;
	}

	.cu-path-row input {
		flex: 1;
	}

	.cu-primary {
		margin-top: 8px;
	}

	.cu-success {
		background: color-mix(in srgb, var(--cu-seal) 15%, transparent);
		border: 1px solid var(--cu-seal);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 16px 0 0;
		overflow-wrap: anywhere;
	}

	.cu-error {
		background: color-mix(in srgb, var(--cu-conflict) 15%, transparent);
		border: 1px solid var(--cu-conflict);
		color: var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 16px 0 0;
		overflow-wrap: anywhere;
	}
</style>
