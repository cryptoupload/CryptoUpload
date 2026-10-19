<script lang="ts">
	import Rich from './Rich.svelte';
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import LanguagePicker from './LanguagePicker.svelte';
	import { openReportBug } from '$lib/state/reportBug.svelte';
	import { onMount, onDestroy } from 'svelte';
	import { friendlyError } from '$lib/errors';
	import { passwordStrength } from '$lib/passwordStrength';
	import { open } from '@tauri-apps/plugin-dialog';
	import {
		vaultCreate,
		vaultUnlock,
		vaultUnlockRecovery,
		vaultPeekUnlockMethods,
		vaultUnlockPlatform,
		vaultUnlockFido2,
		vaultUnlockMember,
		vaultAcceptConfigRollback,
		hasUnlockMethod,
		type UnlockMethod
	} from '$lib/api/vault';
	import { uiSettingsGet, firstLocalVault } from '$lib/api/settings';
	import { session } from '$lib/state/session.svelte';
	import { forcedLockMessage } from '$lib/state/lock';
	import { remoteConsent } from '$lib/state/remoteConsent.svelte';
	import { sceneMotionOff } from '$lib/state/theme.svelte';
	import { createBusyMessages, createMessages } from '$lib/state/busyMessages.svelte';
	import PrivateProfileOption from './PrivateProfileOption.svelte';

	type Mode = 'unlock' | 'create' | 'recovery' | 'remote';

	let mode = $state<Mode>('unlock');
	let root = $state('');
	let password = $state('');
	let passwordConfirm = $state('');
	let privateProfile = $state(false);
	let decoyFiles = $state(false);
	const strength = $derived(passwordStrength(password));
	let words = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);
	let altMethods = $state<UnlockMethod[]>([]);
	let fido2Pin = $state('');
	let memberIdentityPassphrase = $state('');
	let autoLockedNotice = $state(false);
	let forcedLockNotice = $state<string | null>(null);

	const UNLOCK_MESSAGES = $derived([i18n.t.busy.checkingPassword, i18n.t.busy.derivingKey, i18n.t.busy.openingVault]);
	const RECOVERY_MESSAGES = $derived([i18n.t.busy.checkingRecovery, i18n.t.busy.derivingKey, i18n.t.busy.openingVault]);
	const ALT_MESSAGES = $derived([i18n.t.busy.derivingKey, i18n.t.busy.openingVault]);
	const busyMessages = createBusyMessages();
	const stopBusyMessages = busyMessages.stop;

	onMount(async () => {
		autoLockedNotice = session.autoLockedNotice;
		session.autoLockedNotice = false;
		forcedLockNotice = session.forcedLock ? forcedLockMessage(session.forcedLock) : null;
		session.forcedLock = null;

		try {
			const settings = await uiSettingsGet();
			if (settings.reopenLastVault) {
				const lastLocal = firstLocalVault(settings.recentVaults);
				if (lastLocal) root = lastLocal;
			}
		} catch {
		}
		await refreshAltMethods();
	});

	onDestroy(stopBusyMessages);

	async function refreshAltMethods() {
		if (!root.trim()) {
			altMethods = [];
			return;
		}
		try {
			altMethods = await vaultPeekUnlockMethods(root);
		} catch {
			altMethods = [];
		}
	}

	async function chooseFolder() {
		const picked = await open({ directory: true, multiple: false });
		if (typeof picked === 'string') {
			root = picked;
			await refreshAltMethods();
		}
	}

	function resetSecrets() {
		password = '';
		passwordConfirm = '';
		words = '';
		fido2Pin = '';
		memberIdentityPassphrase = '';
	}

	let configRollbackRetry = $state<(() => Promise<void>) | null>(null);

	function noteConfigRollback(e: unknown, retry: () => Promise<void>) {
		configRollbackRetry = String(e) === 'CONFIG_ROLLED_BACK' ? retry : null;
	}

	async function acceptConfigRollback() {
		const retry = configRollbackRetry;
		configRollbackRetry = null;
		if (!retry) return;
		await vaultAcceptConfigRollback();
		await retry();
	}

	async function submit() {
		error = null;
		if (!root.trim()) {
			error = i18n.t.vaultForm.chooseFolderFirst;
			return;
		}
		if (mode === 'create' && password !== passwordConfirm) {
			error = i18n.t.vaultForm.passwordsDiffer;
			return;
		}
		if (mode === 'create' && !password) {
			error = i18n.t.vaultForm.passwordNeeded;
			return;
		}
		busy = true;
		busyMessages.start(
			mode === 'create' ? createMessages() : mode === 'recovery' ? RECOVERY_MESSAGES : UNLOCK_MESSAGES,
		);
		try {
			if (mode === 'unlock') {
				const info = await vaultUnlock(root, password, fido2Pin || undefined);
				session.vaultRoot = root;
				session.vaultInfo = info;
				session.opening = !sceneMotionOff();
				session.screen = 'unlocked';
			} else if (mode === 'recovery') {
				const info = await vaultUnlockRecovery(root, words);
				session.vaultRoot = root;
				session.vaultInfo = info;
				session.opening = !sceneMotionOff();
				session.screen = 'unlocked';
			} else {
				const info = await vaultCreate(root, password, privateProfile, decoyFiles);
				session.vaultRoot = root;
				session.vaultInfo = info;
				session.screen = 'recovery-reveal';
			}
			resetSecrets();
			configRollbackRetry = null;
		} catch (e) {
			error = friendlyError(e);
			noteConfigRollback(e, submit);
		} finally {
			busy = false;
			stopBusyMessages();
		}
	}

	async function submitAlt(unlock: () => Promise<import('$lib/api/vault').VaultInfo>) {
		error = null;
		busy = true;
		busyMessages.start(ALT_MESSAGES);
		try {
			const info = await unlock();
			session.vaultRoot = root;
			session.vaultInfo = info;
			session.opening = !sceneMotionOff();
			session.screen = 'unlocked';
			resetSecrets();
			configRollbackRetry = null;
		} catch (e) {
			error = friendlyError(e);
			noteConfigRollback(e, () => submitAlt(unlock));
		} finally {
			busy = false;
			stopBusyMessages();
		}
	}

	function submitPlatform() {
		return submitAlt(() => vaultUnlockPlatform(root));
	}

	function submitFido2() {
		return submitAlt(() => vaultUnlockFido2(root, fido2Pin || undefined));
	}

	let memberVaultChanged = $state(false);

	function submitMember(acceptNewVault = false) {
		memberVaultChanged = false;
		return submitAlt(async () => {
			try {
				return await vaultUnlockMember(root, memberIdentityPassphrase, acceptNewVault);
			} catch (e) {
				if (String(e) !== 'VAULT_IDENTITY_CHANGED') throw e;
				memberVaultChanged = true;
				throw i18n.t.unlock.differentVault;
			}
		});
	}

	function switchMode(next: Mode) {
		mode = next;
		error = null;
		resetSecrets();
	}
</script>

<div class="cu-gate">
	<div class="cu-card">
		<div class="cu-seal">
			<svg class="cu-i-mark" width="16" height="16" aria-hidden="true"><use href="#cu-i-mark" /></svg>
			CryptoUpload
		</div>
		<h1><Rich text={i18n.t.unlock.headline} /></h1>
		<p class="cu-subtitle">{i18n.t.unlock.subtitle}</p>

		{#if forcedLockNotice}
			<p class="cu-note" role="status">{forcedLockNotice}</p>
		{/if}
		{#if autoLockedNotice}
			<p class="cu-note">
				{i18n.t.unlock.autoLocked}
			</p>
		{/if}

		{#if remoteConsent.programs}
			<label class="cu-remote-consent">
				<input type="checkbox" bind:checked={remoteConsent.allow} />
				<span>
					{fill(i18n.t.unlock.allowRemoteSession, { programs: remoteConsent.programs })}
				</span>
			</label>
		{/if}

		<div class="cu-tabs" role="tablist">
			<button role="tab" aria-selected={mode === 'unlock'} class:active={mode === 'unlock'} onclick={() => switchMode('unlock')}>
				{i18n.t.unlock.tabUnlock}
			</button>
			<button role="tab" aria-selected={mode === 'create'} class:active={mode === 'create'} onclick={() => switchMode('create')}>
				{i18n.t.unlock.tabCreate}
			</button>
			<button role="tab" aria-selected={mode === 'remote'} class:active={mode === 'remote'} onclick={() => switchMode('remote')}>
				{i18n.t.unlock.tabRemote}
			</button>
		</div>

		{#if mode === 'remote'}
			<p class="cu-note">
				{i18n.t.unlock.remoteNote}
			</p>
			<button
				type="button"
				class="cu-primary"
				onclick={() => {
					session.screenBeforeRemotes = 'unlock';
					session.screen = 'remotes';
				}}
			>
				{i18n.t.common.continueMore}
			</button>
		{:else}
			<form onsubmit={(e) => { e.preventDefault(); submit(); }}>
				<label class="cu-field">
					<span>{i18n.t.vaultForm.folder}</span>
					<div class="cu-path-row">
						<input
							class="cu-mono"
							type="text"
							bind:value={root}
							onblur={refreshAltMethods}
							placeholder={i18n.t.vaultForm.folderPlaceholder}
						/>
						<button type="button" class="cu-btn" onclick={chooseFolder}>{i18n.t.common.choose}</button>
					</div>
				</label>

				{#if mode === 'recovery'}
					<label class="cu-field">
						<span>{i18n.t.unlock.recoveryLabel}</span>
						<textarea bind:value={words} rows="3" placeholder={i18n.t.unlock.recoveryPlaceholder}></textarea>
					</label>
					<button type="button" class="cu-link cu-text-link" onclick={() => switchMode('unlock')}>
						{i18n.t.unlock.backToPassword}
					</button>
				{:else}
					<label class="cu-field">
						<span>{i18n.t.vaultForm.password}</span>
						<input class="cu-password-line" type="password" bind:value={password} autocomplete={mode === 'create' ? 'new-password' : 'current-password'} />
					</label>
					{#if mode === 'create'}
						{#if password}
							<p class="cu-strength" data-score={strength.score}>
								{fill(i18n.t.vaultForm.strength, { label: strength.label })}
							</p>
						{/if}
						<label class="cu-field">
							<span>{i18n.t.vaultForm.repeatPassword}</span>
							<input type="password" bind:value={passwordConfirm} autocomplete="new-password" />
						</label>
						<PrivateProfileOption bind:checked={privateProfile} bind:decoys={decoyFiles} />
					{:else}
						{#if hasUnlockMethod(altMethods, 'twoFactor')}
							<p class="cu-note">
								{i18n.t.unlock.secondFactorNote}
							</p>
							<label class="cu-field">
								<span>{i18n.t.unlock.securityKeyPin}</span>
								<input type="password" bind:value={fido2Pin} autocomplete="off" />
							</label>
						{/if}
						<button type="button" class="cu-link cu-text-link" onclick={() => switchMode('recovery')}>
							{i18n.t.unlock.forgotPassword}
						</button>
					{/if}
				{/if}

				{#if mode === 'unlock' && hasUnlockMethod(altMethods, 'platform')}
					<button
						type="button"
						class="cu-alt-unlock cu-btn block"
						disabled={busy}
						onclick={submitPlatform}
					>
						{i18n.t.unlock.withKeychain}
					</button>
				{/if}

				{#if mode === 'unlock' && hasUnlockMethod(altMethods, 'fido2')}
					<label class="cu-field">
						<span>{i18n.t.unlock.securityKeyPin}</span>
						<input type="password" bind:value={fido2Pin} autocomplete="off" />
					</label>
					<button
						type="button"
						class="cu-alt-unlock cu-btn block"
						disabled={busy}
						onclick={submitFido2}
					>
						{i18n.t.unlock.withFido2}
					</button>
				{/if}

				{#if mode === 'unlock' && hasUnlockMethod(altMethods, 'member')}
					<label class="cu-field">
						<span>{i18n.t.unlock.memberPassphrase}</span>
						<input
							type="password"
							bind:value={memberIdentityPassphrase}
							autocomplete="off"
						/>
					</label>
					<button
						type="button"
						class="cu-alt-unlock cu-btn block"
						disabled={busy}
						onclick={() => submitMember()}
					>
						{i18n.t.unlock.asMember}
					</button>
					{#if memberVaultChanged}
						<button
							type="button"
							class="cu-btn block"
							disabled={busy}
							onclick={() => submitMember(true)}
						>
							{i18n.t.unlock.acceptRecreated}
						</button>
					{/if}
				{/if}

				{#if error}
					<p class="cu-error" role="alert">{error}</p>
				{/if}
				{#if configRollbackRetry}
					<button
						type="button"
						class="cu-btn block"
						disabled={busy}
						onclick={acceptConfigRollback}
					>
						{i18n.t.unlock.acceptOldBackup}
					</button>
				{/if}

				<button type="submit" class="cu-primary" disabled={busy}>
					{#if busy}
						{busyMessages.message}
					{:else if mode === 'unlock'}
						{i18n.t.unlock.submitUnlock}
					{:else if mode === 'create'}
						{i18n.t.vaultForm.createVault}
					{:else}
						{i18n.t.unlock.submitRecovery}
					{/if}
				</button>
			</form>
		{/if}

		<button
			type="button"
			class="cu-deposit-link"
			onclick={() => {
				session.screenBeforeDeposit = 'unlock';
				session.screen = 'deposit';
			}}
		>
			{i18n.t.unlock.depositLink}
		</button>
		<button
			type="button"
			class="cu-deposit-link cu-share-link"
			onclick={() => {
				session.screenBeforeOpenShare = 'unlock';
				session.screen = 'open-share';
			}}
		>
			{i18n.t.unlock.openShareLink}
		</button>
		<LanguagePicker compact />
		<button type="button" class="cu-text-link cu-report-link" onclick={openReportBug}>
			{i18n.t.report.title}
		</button>	</div>
</div>

<style>
	.cu-report-link {
		display: block;
		margin: 6px auto 0;
		font-size: 12px;
	}

	.cu-strength {
		margin: -4px 0 10px;
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

	.cu-deposit-link {
		display: block;
		width: 100%;
		margin-top: 20px;
		background: none;
		border: none;
		border-top: 1px solid var(--cu-line);
		padding-top: 16px;
		color: var(--cu-muted);
		font-size: 12px;
		cursor: pointer;
		text-align: center;
	}

	.cu-share-link {
		margin-top: 8px;
		border-top: none;
		padding-top: 0;
	}

	.cu-deposit-link:hover {
		color: var(--cu-text);
	}

	.cu-gate {
		height: 100%;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 32px;
		background: radial-gradient(55% 45% at 50% 50%, var(--cu-key-light), transparent 72%), var(--cu-bg);
	}

	.cu-card {
		width: 100%;
		max-width: 420px;
		padding: 0 6px;
		animation: cu-card-in var(--cu-motion-slow) var(--cu-ease-out) both;
	}

	.cu-seal {
		display: flex;
		align-items: center;
		gap: 8px;
		color: var(--cu-muted);
		font-size: 13px;
		font-weight: 500;
		letter-spacing: 0.06em;
		margin-bottom: 26px;
	}

	.cu-seal .cu-i-mark {
		color: var(--cu-seal-text);
	}

	h1 {
		margin: 0 0 12px;
		font-family: var(--cu-font-display);
		font-weight: 400;
		font-size: 42px;
		line-height: 1.08;
		letter-spacing: -0.005em;
	}

	.cu-subtitle {
		margin: 0 0 30px;
		color: var(--cu-muted);
		font-size: 14px;
		line-height: 1.5;
		max-width: 32ch;
	}

	.cu-tabs {
		display: flex;
		gap: 4px;
		margin-bottom: 20px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-tabs button {
		background: none;
		border: none;
		color: var(--cu-muted);
		padding: 8px 4px 10px;
		cursor: pointer;
		font-size: 13px;
		border-bottom: 2px solid transparent;
		margin-inline-end: 12px;
	}

	.cu-tabs button.active {
		color: var(--cu-text);
		border-bottom-color: var(--cu-seal-text);
	}

	.cu-field {
		display: block;
		margin-bottom: 16px;
	}

	.cu-field > span {
		display: block;
		font-size: 12px;
		color: var(--cu-muted);
		margin-bottom: 6px;
	}

	input,
	textarea {
		width: 100%;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		color: var(--cu-text);
	}

	input {
		height: 34px;
	}

	input.cu-password-line {
		background: none;
		border: none;
		border-bottom: 1px solid var(--cu-line-strong);
		border-radius: 0;
		padding: 8px 0;
		height: auto;
		font-size: 18px;
		letter-spacing: 0.06em;
	}

	input.cu-password-line:focus {
		outline: none;
		border-bottom-color: var(--cu-seal-text);
	}

	input.cu-password-line:focus-visible {
		outline: none;
		border-bottom: 2px solid var(--cu-seal-text);
	}

	textarea {
		padding: 9px 12px;
		resize: vertical;
		font-family: var(--cu-font-mono);
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
		margin-top: 4px;
	}

	.cu-alt-unlock.cu-btn {
		margin: 0 0 12px;
		height: auto;
		min-height: 34px;
		padding-block: 6px;
		white-space: normal;
	}

	.cu-remote-consent {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		font-size: 13px;
		margin: 0 0 12px;
		padding: 10px 12px;
		border: 1px solid var(--cu-warn);
		border-radius: var(--cu-radius-sm);
		text-align: start;
	}

	.cu-remote-consent input {
		width: auto;
		margin-top: 2px;
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

	.cu-note {
		color: var(--cu-muted);
		font-size: 13px;
		margin: 0 0 16px;
		line-height: 1.5;
	}

	.cu-link {
		display: block;
		font-size: 12px;
		text-align: start;
		margin: -8px 0 16px;
	}
</style>
