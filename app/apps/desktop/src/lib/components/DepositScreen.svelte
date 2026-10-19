<script lang="ts">
	import { i18n } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import { open } from '@tauri-apps/plugin-dialog';
	import { basename } from '@tauri-apps/api/path';
	import { session } from '$lib/state/session.svelte';
	import { friendlyError } from '$lib/errors';
	import { remoteListNames, remoteUnlock } from '$lib/api/remotes';
	import { inboxDeposit } from '$lib/api/inbox';

	let remoteNames = $state<string[]>([]);
	let remotesLoaded = $state(false);
	let selectedRemote = $state('');
	let remotePassword = $state('');
	let remoteUnlocked = $state(false);
	let pubkey = $state('');
	let label = $state(i18n.t.deposit.anonymous);
	let filePath = $state('');
	let fileName = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);
	let done = $state(false);

	onMount(async () => {
		try {
			remoteNames = await remoteListNames();
			remotesLoaded = true;
		} catch (e) {
			error = friendlyError(e);
		}
	});

	function onRemoteChange() {
		remoteUnlocked = false;
		remotePassword = '';
		error = null;
	}

	async function unlockRemote() {
		error = null;
		if (!selectedRemote) {
			error = i18n.t.deposit.chooseRemote;
			return;
		}
		if (!remotePassword) {
			error = i18n.t.deposit.remotePasswordNeeded;
			return;
		}
		busy = true;
		try {
			await remoteUnlock(selectedRemote, remotePassword);
			remoteUnlocked = true;
		} catch (e) {
			error = friendlyError(e);
		} finally {
			remotePassword = '';
			busy = false;
		}
	}

	async function pickFile() {
		const picked = await open({ directory: false, multiple: false });
		if (typeof picked !== 'string') return;
		filePath = picked;
		fileName = await basename(picked);
	}

	async function deposit() {
		error = null;
		done = false;
		if (!remoteUnlocked) {
			error = i18n.t.deposit.unlockRemoteFirst;
			return;
		}
		if (!pubkey.trim()) {
			error = i18n.t.deposit.pasteKey;
			return;
		}
		if (!filePath) {
			error = i18n.t.deposit.chooseFile;
			return;
		}
		busy = true;
		try {
			await inboxDeposit(selectedRemote, pubkey.trim(), filePath, label.trim() || i18n.t.deposit.anonymous);
			done = true;
			filePath = '';
			fileName = '';
		} catch (e) {
			error = friendlyError(e);
		} finally {
			busy = false;
		}
	}

	function back() {
		session.screen = session.screenBeforeDeposit;
	}
</script>

<div class="cu-gate">
<div class="cu-deposit">
	<button type="button" class="cu-back cu-btn ghost sm" onclick={back}> {i18n.t.common.back} </button>

	<h1>{i18n.t.deposit.title}</h1>
	<p class="cu-subtitle">
		{i18n.t.deposit.intro}
	</p>

	<label class="cu-field">
		<span>{i18n.t.deposit.remoteLabel}</span>
		<select bind:value={selectedRemote} onchange={onRemoteChange}>
			<option value="" disabled>{i18n.t.deposit.remoteChoose}</option>
			{#each remoteNames as name (name)}
				<option value={name}>{name}</option>
			{/each}
		</select>
	</label>
	{#if remotesLoaded && remoteNames.length === 0}
		<p class="cu-note">
			{i18n.t.deposit.noRemotes}
		</p>
	{/if}

	{#if selectedRemote && !remoteUnlocked}
		<label class="cu-field">
			<span>{i18n.t.deposit.remotePassword}</span>
			<input
				type="password"
				bind:value={remotePassword}
				autocomplete="off"
			/>
		</label>
		<button
			type="button"
			class="cu-btn"
			disabled={busy}
			onclick={unlockRemote}
		>
			{i18n.t.deposit.unlockRemote}
		</button>
	{/if}

	{#if remoteUnlocked}
		<label class="cu-field">
			<span>{i18n.t.deposit.keyLabel}</span>
			<input type="text" bind:value={pubkey} autocomplete="off" />
		</label>

		<label class="cu-field">
			<span>{i18n.t.deposit.labelLabel}</span>
			<input type="text" bind:value={label} />
		</label>

		<label class="cu-field">
			<span>{i18n.t.deposit.fileLabel}</span>
			<div class="cu-path-row">
				<input
					type="text"
					readonly
					class="cu-mono"
					value={fileName}
					placeholder={i18n.t.deposit.noFile}
				/>
				<button type="button" class="cu-btn" onclick={pickFile}>
					{i18n.t.common.choose}
				</button>
			</div>
		</label>

		<button
			type="button"
			class="cu-primary"
			disabled={busy}
			onclick={deposit}
		>
			{busy ? i18n.t.deposit.sending : i18n.t.deposit.submit}
		</button>
	{/if}

	{#if done}
		<p class="cu-success">
			{i18n.t.deposit.done}
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

	.cu-deposit {
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
	}

	.cu-path-row {
		display: flex;
		gap: 8px;
	}

	.cu-path-row input {
		flex: 1;
		min-width: 0;
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
	}

	.cu-error {
		background: color-mix(in srgb, var(--cu-conflict) 15%, transparent);
		border: 1px solid var(--cu-conflict);
		color: var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 16px 0 0;
	}
</style>
