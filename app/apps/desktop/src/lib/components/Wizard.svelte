<script lang="ts">
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import LanguagePicker from './LanguagePicker.svelte';
	import { openReportBug } from '$lib/state/reportBug.svelte';
	import { open } from '@tauri-apps/plugin-dialog';
	import { vaultCreate, vaultLock } from '$lib/api/vault';
	import PrivateProfileOption from './PrivateProfileOption.svelte';
	import { uiSettingsSet } from '$lib/api/settings';
	import { session } from '$lib/state/session.svelte';
	import { friendlyError } from '$lib/errors';
	import { passwordStrength } from '$lib/passwordStrength';
	import { onDestroy } from 'svelte';
	import { createBusyMessages, createMessages } from '$lib/state/busyMessages.svelte';

	let root = $state('');
	let password = $state('');
	let passwordConfirm = $state('');
	let privateProfile = $state(false);
	let decoyFiles = $state(false);
	let busy = $state(false);
	let error = $state<string | null>(null);

	const busyMessages = createBusyMessages();
	onDestroy(busyMessages.stop);

	const strength = $derived(passwordStrength(password));

	async function chooseFolder() {
		const picked = await open({ directory: true, multiple: false });
		if (typeof picked === 'string') {
			root = picked;
		}
	}

	async function markOnboarded() {
		try {
			await uiSettingsSet({ onboarded: true });
		} catch {
		}
	}

	async function leaveOpenVault(): Promise<boolean> {
		if (!session.vaultInfo) return true;
		try {
			await vaultLock();
		} catch (e) {
			error = friendlyError(e);
			return false;
		}
		session.vaultInfo = null;
		session.vaultRoot = '';
		session.section = 'vault';
		session.recoveryPhraseConfirmed = false;
		return true;
	}

	async function chooseOpenExisting() {
		error = null;
		if (!(await leaveOpenVault())) return;
		await markOnboarded();
		session.screen = 'unlock';
	}

	async function chooseRemote() {
		error = null;
		if (!(await leaveOpenVault())) return;
		await markOnboarded();
		session.screenBeforeRemotes = 'unlock';
		session.screen = 'remotes';
	}

	async function submitCreate() {
		error = null;
		if (!root.trim()) {
			error = i18n.t.vaultForm.chooseFolderFirst;
			return;
		}
		if (password !== passwordConfirm) {
			error = i18n.t.vaultForm.passwordsDiffer;
			return;
		}
		if (!password) {
			error = i18n.t.vaultForm.passwordNeeded;
			return;
		}
		busy = true;
		busyMessages.start(createMessages());
		try {
			const info = await vaultCreate(root, password, privateProfile, decoyFiles);
			await markOnboarded();
			session.vaultRoot = root;
			session.vaultInfo = info;
			session.afterRecoveryScreen = 'wizard-shortcuts';
			session.screen = 'recovery-reveal';
		} catch (e) {
			error = friendlyError(e);
		} finally {
			busy = false;
			busyMessages.stop();
			password = '';
			passwordConfirm = '';
		}
	}

	function goToSection(section: 'browse' | 'jobs' | 'vault') {
		session.section = section;
		session.screen = 'unlocked';
	}

	function closeReview() {
		session.screen = 'unlocked';
	}
</script>

<div class="cu-gate">
	<div class="cu-card">
		{#if session.screen === 'wizard-intro'}
			<div class="cu-seal" aria-hidden="true">
				<svg class="cu-i-mark" width="28" height="28"><use href="#cu-i-mark" /></svg>
			</div>
			<h1>{i18n.t.wizard.welcome}</h1>
			<p class="cu-subtitle">
				{i18n.t.wizard.welcomeBody}
			</p>
			<button
				class="cu-primary"
				onclick={() => (session.screen = 'wizard-choice')}
			>
				{i18n.t.common.continue}
			</button>
			{#if session.vaultInfo}
				<button class="cu-skip-link cu-btn ghost sm block" onclick={closeReview}>
					{i18n.t.common.close}
				</button>
			{/if}
		{:else if session.screen === 'wizard-choice'}
			<h1>{i18n.t.wizard.whereToStart}</h1>
			<p class="cu-subtitle">{i18n.t.wizard.changeMind}</p>

			<div class="cu-choice-list">
				<button
					type="button"
					class="cu-choice"
					onclick={() => (session.screen = 'wizard-setup')}
				>
					<strong>{i18n.t.wizard.newVault}</strong>
					<span>{i18n.t.wizard.newVaultDetail}</span>
				</button>
				<button
					type="button"
					class="cu-choice"
					onclick={chooseOpenExisting}
				>
					<strong>{i18n.t.wizard.existingVault}</strong>
					<span>{i18n.t.wizard.existingVaultDetail}</span>
				</button>
				<button
					type="button"
					class="cu-choice"
					onclick={chooseRemote}
				>
					<strong>{i18n.t.wizard.remoteVault}</strong>
					<span>{i18n.t.wizard.remoteVaultDetail}</span>
				</button>
			</div>
			{#if error}
				<p class="cu-error" role="alert">{error}</p>
			{/if}
			{#if session.vaultInfo}
				<button class="cu-skip-link cu-btn ghost sm block" onclick={closeReview}>
					{i18n.t.common.close}
				</button>
			{/if}
		{:else if session.screen === 'wizard-setup'}
			<h1>{i18n.t.wizard.createTitle}</h1>
			<p class="cu-subtitle">
				{i18n.t.wizard.createWarning}
			</p>

			<form
				onsubmit={(e) => {
					e.preventDefault();
					submitCreate();
				}}
			>
				<label class="cu-field">
					<span>{i18n.t.vaultForm.folder}</span>
					<div class="cu-path-row">
						<input
							class="cu-mono"
							type="text"
							bind:value={root}
							placeholder={i18n.t.vaultForm.folderPlaceholder}
						/>
						<button type="button" class="cu-btn" onclick={chooseFolder}>
							{i18n.t.common.choose}
						</button>
					</div>
				</label>

				<label class="cu-field">
					<span>{i18n.t.vaultForm.password}</span>
					<input
						type="password"
						autocomplete="new-password"
						bind:value={password}
					/>
				</label>
				{#if password}
					<p class="cu-strength" data-score={strength.score}>
						{fill(i18n.t.vaultForm.strength, { label: strength.label })}
					</p>
				{/if}

				<label class="cu-field">
					<span>{i18n.t.vaultForm.repeatPassword}</span>
					<input
						type="password"
						autocomplete="new-password"
						bind:value={passwordConfirm}
					/>
				</label>

				<PrivateProfileOption bind:checked={privateProfile} bind:decoys={decoyFiles} />

				{#if error}
					<p class="cu-error" role="alert">{error}</p>
				{/if}

				<button class="cu-primary" type="submit" disabled={busy}>
					{busy ? busyMessages.message : i18n.t.vaultForm.createVault}
				</button>
			</form>
			<button
				type="button"
				class="cu-skip-link cu-btn ghost sm block"
				disabled={busy}
				onclick={() => {
					error = null;
					session.screen = 'wizard-choice';
				}}
			>
				{i18n.t.common.backPlain}
			</button>
		{:else if session.screen === 'wizard-shortcuts'}
			<h1>{i18n.t.wizard.ready}</h1>
			<p class="cu-subtitle">{i18n.t.wizard.fromHere}</p>

			<div class="cu-choice-list">
				<button
					type="button"
					class="cu-choice"
					onclick={() => goToSection('browse')}
				>
					<strong>{i18n.t.wizard.uploadFile}</strong>
					<span>{i18n.t.wizard.uploadFileDetail}</span>
				</button>
				<button
					type="button"
					class="cu-choice"
					onclick={() => goToSection('jobs')}
				>
					<strong>{i18n.t.wizard.keepSynced}</strong>
					<span>{i18n.t.wizard.keepSyncedDetail}</span>
				</button>
				<button
					type="button"
					class="cu-choice"
					onclick={() => goToSection('vault')}
				>
					<strong>{i18n.t.wizard.mountDisk}</strong>
					<span>{i18n.t.wizard.mountDiskDetail}</span>
				</button>
			</div>

			<button
				type="button"
				class="cu-skip-link cu-btn ghost sm block"
				onclick={() => goToSection('vault')}
			>
				{i18n.t.wizard.skipShortcuts}
			</button>
		{/if}
		{#if session.screen === 'wizard-intro'}
			<LanguagePicker compact />
			<button type="button" class="cu-text-link cu-report-link" onclick={openReportBug}>
			{i18n.t.report.title}
		</button>		{/if}
	</div>
</div>

<style>
	.cu-report-link {
		display: block;
		margin: 6px auto 0;
		font-size: 12px;
	}

	.cu-gate {
		height: 100%;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 32px;
	}

	.cu-card {
		width: 100%;
		max-width: 460px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		box-shadow: var(--cu-shadow);
		padding: 32px 28px;
		animation: cu-card-in var(--cu-motion-slow) var(--cu-ease-out) both;
	}

	.cu-seal {
		color: var(--cu-seal-text);
		margin-bottom: 8px;
	}

	h1 {
		margin: 0 0 2px;
		font-family: var(--cu-font-display);
		font-weight: 400;
		font-size: 26px;
		letter-spacing: -0.005em;
	}

	.cu-subtitle {
		margin: 0 0 24px;
		color: var(--cu-muted);
		font-size: 13px;
		line-height: 1.6;
	}

	.cu-choice-list {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}

	.cu-choice {
		display: flex;
		flex-direction: column;
		gap: 4px;
		text-align: start;
		background: var(--cu-surface-raised);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 14px 16px;
		cursor: pointer;
		color: var(--cu-text);
	}

	.cu-choice:hover {
		border-color: var(--cu-seal);
	}

	.cu-choice strong {
		font-size: 14px;
	}

	.cu-choice span {
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-field {
		display: block;
		margin: 0 0 14px;
	}

	.cu-field span {
		display: block;
		font-size: 12px;
		color: var(--cu-muted);
		margin-bottom: 4px;
	}

	.cu-field input {
		width: 100%;
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		color: var(--cu-text);
		padding: 0 12px;
		font-size: 14px;
	}

	.cu-path-row {
		display: flex;
		gap: 8px;
	}

	.cu-path-row input {
		flex: 1;
	}

	.cu-strength {
		margin: -8px 0 14px;
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-strength[data-score='0'] {
		color: var(--cu-conflict);
	}

	.cu-strength[data-score='2'],
	.cu-strength[data-score='3'] {
		color: var(--cu-verified);
	}

	.cu-error {
		background: color-mix(in srgb, var(--cu-conflict) 15%, transparent);
		border: 1px solid var(--cu-conflict);
		color: var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 0 0 16px;
	}

	.cu-skip-link.cu-btn {
		margin-top: 14px;
		height: auto;
		min-height: 28px;
		padding-block: 5px;
		white-space: normal;
		text-align: center;
	}
</style>
