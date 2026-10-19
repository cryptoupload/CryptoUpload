<script lang="ts">
	import Rich from './Rich.svelte';
	import { fill, i18n, plural } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import { uiSettingsGet, uiSettingsSet, type Motion, type Theme } from '$lib/api/settings';
	import { theme, setTheme, setMotion } from '$lib/state/theme.svelte';
	import LanguagePicker from './LanguagePicker.svelte';
	import { setAutoLockMinutes } from '$lib/state/autolock.svelte';
	import { setNotificationsEnabled } from '$lib/state/notifications.svelte';
	import { showIntroScene } from '$lib/state/intro.svelte';
	import { session } from '$lib/state/session.svelte';
	import { friendlyError, describeMessage } from '$lib/errors';
	import { passwordStrength } from '$lib/passwordStrength';
	import {
		vaultChangePassword,
		vaultUnlockMethods,
		vaultEnablePlatformUnlock,
		vaultDisablePlatformUnlock,
		vaultEnableFido2,
		vaultDisableFido2,
		recoverySplit,
		vaultEnableTwoFactor,
		vaultDisableTwoFactor,
		vaultChangePasswordTwoFactor,
		hasUnlockMethod,
		type UnlockMethod,
		memberIdentityExists,
		memberIdentityGenerate,
		memberIdentityPublicKey,
		vaultListMembers,
		vaultAddMember,
		vaultRemoveMember,
			type MemberInfo,
		vaultInboxStatus,
		vaultEnableInbox,
		vaultDisableInbox,
		vaultListInbox,
		vaultCollectInbox,
		type InboxListing,
		type CollectReport
	} from '$lib/api/vault';
	import { recoveryKitState, generateRecoveryKit } from '$lib/state/recoveryKit.svelte';
	import { vaultRegistryReset, vaultRegistryVerify, type RegistryVerifyResult } from '$lib/api/registry';
	import SecurityAdvanced from './SecurityAdvanced.svelte';
	import ProxySetting from './ProxySetting.svelte';
	import type { SecurityState } from '$lib/api/security';

	const motionOptions = $derived<{ value: Motion; label: string }[]>([
		{ value: 'full', label: i18n.t.settings.motionFull },
		{ value: 'reduced', label: i18n.t.settings.motionReduced },
		{ value: 'off', label: i18n.t.settings.motionOff },
	]);
	const themeOptions = $derived<{ value: Theme; label: string }[]>([
		{ value: 'system', label: i18n.t.settings.themeSystem },
		{ value: 'light', label: i18n.t.settings.themeLight },
		{ value: 'dark', label: i18n.t.settings.themeDark },
	]);

	function duration(value: number, unit: 'minute' | 'hour'): string {
		return new Intl.NumberFormat(i18n.language.tag, { style: 'unit', unit, unitDisplay: 'long' }).format(value);
	}
	const autoLockOptions = $derived<{ value: number; label: string }[]>([
		{ value: 0, label: i18n.t.settings.never },
		...[1, 5, 15, 30].map((m) => ({ value: m, label: duration(m, 'minute') })),
		{ value: 60, label: duration(1, 'hour') },
	]);

	let registryBusy = $state(false);
	let registryResult = $state<RegistryVerifyResult | null>(null);
	let registryError = $state<string | null>(null);

	async function runRegistry(register: boolean, seal: boolean): Promise<void> {
		if (registryBusy) return;
		registryBusy = true;
		registryError = null;
		try {
			registryResult = await vaultRegistryVerify(register, seal);
		} catch (e) {
			registryResult = null;
			registryError = friendlyError(e);
		} finally {
			registryBusy = false;
		}
	}

	let confirmingRegistryReset = $state(false);
	let registryResetDone = $state<number | null>(null);

	async function resetRegistry(): Promise<void> {
		if (registryBusy) return;
		registryBusy = true;
		registryError = null;
		registryResetDone = null;
		try {
			registryResult = await vaultRegistryReset();
			registryResetDone = registryResult.registered;
			confirmingRegistryReset = false;
		} catch (e) {
			registryResult = null;
			registryError = friendlyError(e);
		} finally {
			registryBusy = false;
		}
	}

	let loaded = $state(false);
	let autoLockMinutes = $state(15);
	let reopenLastVault = $state(true);
	let autoUpdateCheck = $state(true);
	let runInBackground = $state(true);
	let startAtLogin = $state(false);
	let notificationsEnabled = $state(true);
	let bandwidthLimitKib = $state(0);

	let newPassword = $state('');
	let newPasswordRepeat = $state('');
	let changingPassword = $state(false);
	let passwordError = $state<string | null>(null);
	let passwordChanged = $state(false);
	const newPasswordStrength = $derived(passwordStrength(newPassword));

	async function changePassword() {
		passwordError = null;
		passwordChanged = false;
		if (newPassword.length === 0) {
			passwordError = i18n.t.settings.newPasswordNeeded;
			return;
		}
		if (newPassword !== newPasswordRepeat) {
			passwordError = i18n.t.vaultForm.passwordsDiffer;
			return;
		}
		changingPassword = true;
		try {
			if (twoFactorOn) {
				await vaultChangePasswordTwoFactor(currentPassword, newPassword, twoFactorPin || undefined);
				currentPassword = '';
			} else {
				await vaultChangePassword(newPassword);
			}
			newPassword = '';
			newPasswordRepeat = '';
			passwordChanged = true;
		} catch (e) {
			passwordError = friendlyError(e);
		} finally {
			changingPassword = false;
		}
	}

	let unlockMethods = $state<UnlockMethod[]>([]);
	let unlockMethodsError = $state<string | null>(null);
	let unlockMethodsBusy = $state(false);
	let fido2EnablePin = $state('');

	const twoFactorOn = $derived(hasUnlockMethod(unlockMethods, 'twoFactor'));
	let twoFactorPassword = $state('');
	let twoFactorPin = $state('');
	let currentPassword = $state('');

	let splitWords = $state('');
	let splitThreshold = $state(2);
	let splitCount = $state(3);
	let splitPieces = $state<string[]>([]);
	let splitError = $state<string | null>(null);
	let splitBusy = $state(false);

	async function splitRecovery() {
		splitError = null;
		splitPieces = [];
		if (splitThreshold > splitCount) {
			splitError = i18n.t.settings.splitTooMany;
			return;
		}
		splitBusy = true;
		try {
			splitPieces = await recoverySplit(splitWords, splitThreshold, splitCount);
			splitWords = '';
		} catch (e) {
			splitError = friendlyError(e);
		} finally {
			splitBusy = false;
		}
	}

	async function toggleTwoFactor() {
		unlockMethodsError = null;
		if (!twoFactorPassword) {
			unlockMethodsError = twoFactorOn
				? i18n.t.settings.futurePasswordNeeded
				: i18n.t.settings.vaultPasswordWithKey;
			return;
		}
		unlockMethodsBusy = true;
		try {
			if (twoFactorOn) {
				await vaultDisableTwoFactor(twoFactorPassword);
			} else {
				await vaultEnableTwoFactor(twoFactorPassword, twoFactorPin || undefined);
			}
			twoFactorPassword = '';
			twoFactorPin = '';
			await refreshUnlockMethods();
		} catch (e) {
			unlockMethodsError = friendlyError(e);
		} finally {
			unlockMethodsBusy = false;
		}
	}

	let memberIdentityExistsFlag = $state(false);
	let memberIdentityPassphrase = $state('');
	let memberIdentityPassphraseConfirm = $state('');
	let memberPublicKey = $state<string | null>(null);
	let memberIdentityError = $state<string | null>(null);
	let memberIdentityBusy = $state(false);

	let members = $state<MemberInfo[]>([]);
	let membersError = $state<string | null>(null);
	let membersBusy = $state(false);
	let newMemberName = $state('');
	let newMemberPubkey = $state('');
	let confirmingRemoveMember = $state<string | null>(null);
	let removeMemberPassword = $state('');
	let removeMemberUseFido2 = $state(false);
	let removeMemberPin = $state('');

	let inboxEnabled = $state(false);
	let inboxPublicKey = $state<string | null>(null);
	let inboxDeposits = $state<InboxListing[]>([]);
	let inboxCollectReport = $state<CollectReport | null>(null);
	let inboxError = $state<string | null>(null);
	let inboxBusy = $state(false);

	let settingsError = $state<string | null>(null);

	async function saveSettings(patch: Parameters<typeof uiSettingsSet>[0]) {
		settingsError = null;
		try {
			await uiSettingsSet(patch);
		} catch (e) {
			settingsError = fill(i18n.t.settings.notSaved, { error: friendlyError(e) });
		}
	}

	onMount(async () => {
		try {
			const settings = await uiSettingsGet();
			autoLockMinutes = settings.autoLockMinutes;
			reopenLastVault = settings.reopenLastVault;
			autoUpdateCheck = settings.autoUpdateCheck;
			runInBackground = settings.runInBackground;
			startAtLogin = settings.startAtLogin;
			notificationsEnabled = settings.notificationsEnabled;
			bandwidthLimitKib = settings.bandwidthLimitKib;
			loaded = true;
		} catch (e) {
			settingsError = fill(i18n.t.settings.loadFailed, { error: friendlyError(e) });
			return;
		}
		await refreshUnlockMethods();
		await refreshMemberIdentityState();
		await refreshMembers();
		await refreshInboxStatus();
	});

	async function refreshUnlockMethods() {
		if (!session.vaultInfo) {
			unlockMethods = [];
			return;
		}
		try {
			unlockMethods = await vaultUnlockMethods();
		} catch {
			unlockMethods = [];
		}
	}

	let shortcutsForbidden = $state(false);

	function onSecurityChanged(next: SecurityState) {
		shortcutsForbidden = next.effective.noUnlockShortcuts;
		void refreshUnlockMethods();
	}

	async function togglePlatformUnlock() {
		unlockMethodsError = null;
		unlockMethodsBusy = true;
		try {
			if (hasUnlockMethod(unlockMethods, 'platform')) {
				await vaultDisablePlatformUnlock();
			} else {
				await vaultEnablePlatformUnlock();
			}
			await refreshUnlockMethods();
		} catch (e) {
			unlockMethodsError = friendlyError(e);
		} finally {
			unlockMethodsBusy = false;
		}
	}

	async function toggleFido2() {
		unlockMethodsError = null;
		unlockMethodsBusy = true;
		try {
			if (hasUnlockMethod(unlockMethods, 'fido2')) {
				await vaultDisableFido2();
			} else {
				await vaultEnableFido2(fido2EnablePin || undefined);
			}
			fido2EnablePin = '';
			await refreshUnlockMethods();
		} catch (e) {
			unlockMethodsError = friendlyError(e);
		} finally {
			unlockMethodsBusy = false;
		}
	}

	async function refreshMemberIdentityState() {
		try {
			memberIdentityExistsFlag = await memberIdentityExists();
		} catch {
			memberIdentityExistsFlag = false;
		}
	}

	async function generateMemberIdentity() {
		memberIdentityError = null;
		if (!memberIdentityPassphrase) {
			memberIdentityError = i18n.t.settings.identityPassphraseNeeded;
			return;
		}
		if (memberIdentityPassphrase !== memberIdentityPassphraseConfirm) {
			memberIdentityError = i18n.t.settings.passphrasesDiffer;
			return;
		}
		memberIdentityBusy = true;
		try {
			memberPublicKey = await memberIdentityGenerate(memberIdentityPassphrase);
			memberIdentityExistsFlag = true;
		} catch (e) {
			memberIdentityError = friendlyError(e);
		} finally {
			memberIdentityPassphrase = '';
			memberIdentityPassphraseConfirm = '';
			memberIdentityBusy = false;
		}
	}

	async function showMemberPublicKey() {
		memberIdentityError = null;
		if (!memberIdentityPassphrase) {
			memberIdentityError = i18n.t.settings.identityPassphraseEnter;
			return;
		}
		memberIdentityBusy = true;
		try {
			memberPublicKey = await memberIdentityPublicKey(memberIdentityPassphrase);
		} catch (e) {
			memberIdentityError = friendlyError(e);
		} finally {
			memberIdentityPassphrase = '';
			memberIdentityBusy = false;
		}
	}

	async function copyMemberPublicKey() {
		if (!memberPublicKey) return;
		try {
			await navigator.clipboard.writeText(memberPublicKey);
		} catch {
		}
	}

	async function refreshMembers() {
		if (!session.vaultInfo) {
			members = [];
			return;
		}
		try {
			members = await vaultListMembers();
		} catch {
			members = [];
		}
	}

	async function addMember() {
		membersError = null;
		if (!newMemberName.trim()) {
			membersError = i18n.t.settings.memberNameNeeded;
			return;
		}
		if (!newMemberPubkey.trim()) {
			membersError = i18n.t.settings.memberKeyNeeded;
			return;
		}
		membersBusy = true;
		try {
			await vaultAddMember(newMemberName.trim(), newMemberPubkey.trim());
			newMemberName = '';
			newMemberPubkey = '';
			await refreshMembers();
		} catch (e) {
			membersError = friendlyError(e);
		} finally {
			membersBusy = false;
		}
	}

	function startRemoveMember(name: string) {
		confirmingRemoveMember = name;
		removeMemberPassword = '';
		removeMemberUseFido2 = false;
		removeMemberPin = '';
		membersError = null;
	}

	async function removeMember(name: string) {
		if (!removeMemberPassword) {
			membersError = i18n.t.settings.vaultPasswordToConfirm;
			return;
		}
		membersError = null;
		membersBusy = true;
		try {
			const result = await vaultRemoveMember(
				name,
				removeMemberPassword,
				removeMemberUseFido2,
				removeMemberPin
			);
			session.pendingRemoveMemberResult = result;
			confirmingRemoveMember = null;
			if (result.newRecoveryPhrase && session.vaultInfo) {
				session.vaultInfo = { ...session.vaultInfo, hasRecoveryPhrase: true };
			}
			await refreshMembers();
			await refreshUnlockMethods();
		} catch (e) {
			membersError = friendlyError(e);
		} finally {
			removeMemberPassword = '';
			removeMemberPin = '';
			membersBusy = false;
		}
	}

	async function refreshInboxStatus() {
		if (!session.vaultInfo) {
			inboxEnabled = false;
			inboxPublicKey = null;
			inboxDeposits = [];
			return;
		}
		try {
			const [enabled, publicKey] = await vaultInboxStatus();
			inboxEnabled = enabled;
			inboxPublicKey = publicKey;
			if (enabled) await refreshInboxDeposits();
		} catch {
			inboxEnabled = false;
			inboxPublicKey = null;
			inboxDeposits = [];
		}
	}

	async function refreshInboxDeposits() {
		try {
			inboxDeposits = await vaultListInbox();
		} catch (e) {
			inboxError = friendlyError(e);
		}
	}

	async function enableInbox() {
		inboxError = null;
		inboxBusy = true;
		try {
			inboxPublicKey = await vaultEnableInbox();
			inboxEnabled = true;
			await refreshInboxDeposits();
		} catch (e) {
			inboxError = friendlyError(e);
		} finally {
			inboxBusy = false;
		}
	}

	async function disableInbox() {
		inboxError = null;
		inboxBusy = true;
		try {
			await vaultDisableInbox();
			inboxEnabled = false;
			inboxPublicKey = null;
		} catch (e) {
			inboxError = friendlyError(e);
		} finally {
			inboxBusy = false;
		}
	}

	async function copyInboxPublicKey() {
		if (!inboxPublicKey) return;
		try {
			await navigator.clipboard.writeText(inboxPublicKey);
		} catch {
		}
	}

	async function collectInbox() {
		inboxError = null;
		inboxBusy = true;
		try {
			inboxCollectReport = await vaultCollectInbox(2048, 20 * 1024);
			await refreshInboxDeposits();
		} catch (e) {
			inboxError = friendlyError(e);
		} finally {
			inboxBusy = false;
		}
	}

	async function onAutoLockChange(value: number) {
		autoLockMinutes = value;
		setAutoLockMinutes(value);
		await saveSettings({ autoLockMinutes: value });
	}

	async function onReopenLastVaultChange(value: boolean) {
		reopenLastVault = value;
		await saveSettings({ reopenLastVault: value });
	}

	async function onAutoUpdateCheckChange(value: boolean) {
		autoUpdateCheck = value;
		await saveSettings({ autoUpdateCheck: value });
	}

	async function onRunInBackgroundChange(value: boolean) {
		runInBackground = value;
		await saveSettings({ runInBackground: value });
	}

	async function onStartAtLoginChange(value: boolean) {
		settingsError = null;
		try {
			await uiSettingsSet({ startAtLogin: value });
			startAtLogin = value;
		} catch (e) {
			startAtLogin = !value;
			settingsError = fill(i18n.t.settings.notSaved, { error: friendlyError(e) });
		}
	}

	async function onNotificationsEnabledChange(value: boolean) {
		notificationsEnabled = value;
		setNotificationsEnabled(value);
		await saveSettings({ notificationsEnabled: value });
	}

	async function onBandwidthLimitChange(value: number) {
		bandwidthLimitKib = Math.max(0, Math.round(value || 0));
		await saveSettings({ bandwidthLimitKib });
	}
</script>

<div class="cu-page">
	<h1 class="cu-page-title">{i18n.t.nav.settings}</h1>

	{#if settingsError}
		<p class="cu-error" role="alert">{settingsError}</p>
	{/if}

	<section class="cu-section">
		<h2>{i18n.t.settings.appearance}</h2>
		<div class="cu-theme-row">
			<span class="cu-row-label">{i18n.t.settings.theme}</span>
			<div class="cu-theme-switch" role="radiogroup" aria-label={i18n.t.settings.theme}>
				{#each themeOptions as opt (opt.value)}
					<button
						type="button"
						role="radio"
						aria-checked={theme.current === opt.value}
						class:active={theme.current === opt.value}
						onclick={() => setTheme(opt.value)}
					>
						{opt.label}
					</button>
				{/each}
			</div>
		</div>
		<div class="cu-theme-row">
			<span class="cu-row-label">{i18n.t.settings.motion}</span>
			<div class="cu-theme-switch" role="radiogroup" aria-label={i18n.t.settings.motion}>
				{#each motionOptions as opt (opt.value)}
					<button
						type="button"
						role="radio"
						aria-checked={theme.motion === opt.value}
						class:active={theme.motion === opt.value}
						onclick={() => setMotion(opt.value)}
					>
						{opt.label}
					</button>
				{/each}
			</div>
		</div>
		<div class="cu-language-row">
			<LanguagePicker />
		</div>
	</section>

	{#if loaded}
		<section class="cu-section">
			<h2>{i18n.t.settings.security}</h2>
			<label class="cu-field cu-inline-field">
				<span>{i18n.t.settings.autoLockAfter}</span>
				<select
					value={autoLockMinutes}
					onchange={(e) => onAutoLockChange(Number(e.currentTarget.value))}
				>
					{#each autoLockOptions as opt (opt.value)}
						<option value={opt.value}>{opt.label}</option>
					{/each}
				</select>
			</label>
			<p class="cu-note">
				{i18n.t.settings.autoLockNote}
			</p>
		</section>

		<section class="cu-section">
			<h2>{i18n.t.settings.startup}</h2>
			<label class="cu-checkbox">
				<input
					type="checkbox"
					checked={reopenLastVault}
					onchange={(e) => onReopenLastVaultChange(e.currentTarget.checked)}
				/>
				<span>{i18n.t.settings.reopenLast}</span>
			</label>
			<label class="cu-checkbox">
				<input
					type="checkbox"
					checked={autoUpdateCheck}
					onchange={(e) => onAutoUpdateCheckChange(e.currentTarget.checked)}
				/>
				<span>{i18n.t.settings.autoUpdateCheck}</span>
			</label>
			<label class="cu-checkbox">
				<input
					type="checkbox"
					checked={runInBackground}
					onchange={(e) => onRunInBackgroundChange(e.currentTarget.checked)}
				/>
				<span>{i18n.t.settings.runInBackground}</span>
			</label>
			<p class="cu-note">{i18n.t.settings.runInBackgroundNote}</p>
			<label class="cu-checkbox">
				<input
					type="checkbox"
					checked={startAtLogin}
					onchange={(e) => onStartAtLoginChange(e.currentTarget.checked)}
				/>
				<span>{i18n.t.settings.startAtLogin}</span>
			</label>
			<p class="cu-note">{i18n.t.settings.startAtLoginNote}</p>
			<button type="button" class="cu-link cu-text-link" onclick={showIntroScene}>
				{i18n.t.settings.reviewIntro}
			</button>
		</section>

		<section class="cu-section">
			<h2>{i18n.t.settings.notifications}</h2>
			<label class="cu-checkbox">
				<input
					type="checkbox"
					checked={notificationsEnabled}
					onchange={(e) => onNotificationsEnabledChange(e.currentTarget.checked)}
				/>
				<span>{i18n.t.settings.notificationsToggle}</span>
			</label>
		</section>

		<section class="cu-section">
			<h2>{i18n.t.settings.network}</h2>
			<label class="cu-field cu-inline-field">
				<span>{i18n.t.settings.bandwidth}</span>
				<input
					type="number"
					min="0"
					step="1"
					value={bandwidthLimitKib}
					onchange={(e) => onBandwidthLimitChange(Number(e.currentTarget.value))}
				/>
			</label>
			<p class="cu-note">
				{i18n.t.settings.bandwidthNote}
			</p>
		</section>

		{#if session.vaultInfo}
			<SecurityAdvanced onchanged={onSecurityChanged} />
			<ProxySetting />

			<section class="cu-section">
				<h2>{i18n.t.vaultForm.password}</h2>
				<p class="cu-note">
					{i18n.t.settings.changePasswordNote}
				</p>
				<form
					onsubmit={(e) => {
						e.preventDefault();
						changePassword();
					}}
				>
					{#if twoFactorOn}
						<p class="cu-note">
							{i18n.t.settings.changePasswordSecondFactor}
						</p>
						<label class="cu-field">
							<span>{i18n.t.settings.currentPassword}</span>
							<input
								type="password"
								autocomplete="current-password"
								bind:value={currentPassword}
							/>
						</label>
					{/if}
					<label class="cu-field">
						<span>{i18n.t.settings.newPassword}</span>
						<input
							type="password"
							autocomplete="new-password"
							bind:value={newPassword}
						/>
					</label>
					{#if newPassword}
						<p
							class="cu-strength"
							data-score={newPasswordStrength.score}
						>
							{fill(i18n.t.vaultForm.strength, { label: newPasswordStrength.label })}
						</p>
					{/if}
					<label class="cu-field">
						<span>{i18n.t.settings.repeatNewPassword}</span>
						<input
							type="password"
							autocomplete="new-password"
							bind:value={newPasswordRepeat}
						/>
					</label>
					<button
						type="submit"
						class="cu-btn"
						disabled={changingPassword || !newPassword}
					>
						{changingPassword ? i18n.t.settings.changing : i18n.t.settings.changePassword}
					</button>
				</form>
				{#if passwordError}
					<p class="cu-error" role="alert">
						{passwordError}
					</p>
				{/if}
				{#if passwordChanged}
					<p class="cu-note" role="status">
						{i18n.t.settings.passwordChanged}
					</p>
				{/if}
			</section>

			<section class="cu-section">
				<h2>{i18n.t.settings.unlockMethods}</h2>
				<p class="cu-note">
					{i18n.t.settings.unlockMethodsNote}
				</p>

				<div class="cu-unlock-method-row">
					<div class="cu-unlock-method-text">
						<strong>{i18n.t.settings.keychain}</strong>
						<p class="cu-note">
							<Rich text={i18n.t.settings.keychainNote} />
						</p>
						{#if shortcutsForbidden}
							<p class="cu-note">
								{i18n.t.settings.keychainDisabledByPolicy}
							</p>
						{/if}
					</div>
					<button
						type="button"
						class="cu-btn"
						disabled={unlockMethodsBusy || shortcutsForbidden}
						onclick={togglePlatformUnlock}
					>
						{hasUnlockMethod(unlockMethods, 'platform') ? i18n.t.settings.turnOff : i18n.t.settings.turnOn}
					</button>
				</div>

				<div class="cu-unlock-method-row">
					<div class="cu-unlock-method-text">
						<strong>{i18n.t.settings.fido2}</strong>
						<p class="cu-note">
							{i18n.t.settings.fido2Note}
						</p>
						{#if !hasUnlockMethod(unlockMethods, 'fido2')}
							<label class="cu-field">
								<span>{i18n.t.security.keyPinPlaceholder}</span>
								<input
									type="password"
									bind:value={fido2EnablePin}
									autocomplete="off"
								/>
							</label>
						{/if}
					</div>
					<button
						type="button"
						class="cu-btn"
						disabled={unlockMethodsBusy}
						onclick={toggleFido2}
					>
						{hasUnlockMethod(unlockMethods, 'fido2') ? i18n.t.common.removeEntry : i18n.t.settings.register}
					</button>
				</div>

				<div class="cu-unlock-method-row">
					<div class="cu-unlock-method-text">
						<strong>{i18n.t.settings.secondFactor}</strong>
						<p class="cu-note">
							{#if twoFactorOn}
								<Rich text={i18n.t.settings.secondFactorOn} />
							{:else}
								<Rich text={i18n.t.settings.secondFactorOff} />
							{/if}
						</p>
						{#if !twoFactorOn && !session.vaultInfo?.hasRecoveryPhrase}
							<p class="cu-note">
								<Rich text={i18n.t.settings.secondFactorNoRecovery} />
							</p>
						{/if}
						<label class="cu-field">
							<span>{twoFactorOn ? i18n.t.settings.futurePassword : i18n.t.vaultForm.password}</span>
							<input
								type="password"
								bind:value={twoFactorPassword}
								autocomplete="off"
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.security.keyPinPlaceholder}</span>
							<input
								type="password"
								bind:value={twoFactorPin}
								autocomplete="off"
							/>
						</label>
					</div>
					<button
						type="button"
						class="cu-btn"
						disabled={unlockMethodsBusy}
						onclick={toggleTwoFactor}
					>
						{#if unlockMethodsBusy}{i18n.t.common.working}{:else if twoFactorOn}{i18n.t.settings.turnOff}{:else}{i18n.t.settings.turnOn}{/if}
					</button>
				</div>

				{#if unlockMethodsError}
					<p class="cu-error" role="alert">{unlockMethodsError}</p>
				{/if}
			</section>

			<section class="cu-section">
				<h2>{i18n.t.settings.split}</h2>
				<p class="cu-note">
					{i18n.t.settings.splitNote}
				</p>
				{#if !session.vaultInfo.hasRecoveryPhrase}
					<p class="cu-note">
						{i18n.t.settings.splitNoRecovery}
					</p>
				{:else}
					<label class="cu-field">
						<span>{i18n.t.settings.splitWords}</span>
						<textarea
							rows="3"
							bind:value={splitWords}
							placeholder={i18n.t.unlock.recoveryPlaceholder}
						></textarea>
					</label>
					<div class="cu-split-row">
						<label class="cu-field cu-inline-field">
							<span>{i18n.t.settings.splitCount}</span>
							<select bind:value={splitCount}>
								{#each [2, 3, 4, 5, 6, 7, 8] as n (n)}
									<option value={n}>{n}</option>
								{/each}
							</select>
						</label>
						<label class="cu-field cu-inline-field">
							<span>{i18n.t.settings.splitThreshold}</span>
							<select bind:value={splitThreshold}>
								{#each [2, 3, 4, 5, 6, 7, 8] as n (n)}
									<option value={n}>{n}</option>
								{/each}
							</select>
						</label>
						<button
							type="button"
							class="cu-btn"
							disabled={splitBusy || !splitWords.trim()}
							onclick={splitRecovery}
						>
							{splitBusy ? i18n.t.settings.splitting : i18n.t.settings.splitSubmit}
						</button>
					</div>
					{#if splitError}
						<p class="cu-error" role="alert">{splitError}</p>
					{/if}
					{#if splitPieces.length > 0}
						<p class="cu-note" role="status">
							{i18n.t.settings.splitResultNote}
						</p>
						<ol class="cu-split-pieces">
							{#each splitPieces as piece, index (index)}
								<li><span class="cu-mono">{piece}</span></li>
							{/each}
						</ol>
					{/if}
				{/if}
			</section>
		{/if}

		<section class="cu-section">
			<h2>{i18n.t.settings.identity}</h2>
			<p class="cu-note">
				{i18n.t.settings.identityNote}
			</p>

			{#if !memberIdentityExistsFlag}
				<label class="cu-field">
					<span>{i18n.t.settings.identityChoosePassphrase}</span>
					<input
						type="password"
						bind:value={memberIdentityPassphrase}
						autocomplete="off"
					/>
				</label>
				<label class="cu-field">
					<span>{i18n.t.settings.repeatIt}</span>
					<input
						type="password"
						bind:value={memberIdentityPassphraseConfirm}
						autocomplete="off"
					/>
				</label>
				<button
					type="button"
					class="cu-btn"
					disabled={memberIdentityBusy}
					onclick={generateMemberIdentity}
				>
					{i18n.t.settings.identityGenerate}
				</button>
			{:else if !memberPublicKey}
				<label class="cu-field">
					<span>{i18n.t.settings.identityPassphrase}</span>
					<input
						type="password"
						bind:value={memberIdentityPassphrase}
						autocomplete="off"
					/>
				</label>
				<button
					type="button"
					class="cu-btn"
					disabled={memberIdentityBusy}
					onclick={showMemberPublicKey}
				>
					{i18n.t.settings.identityShowKey}
				</button>
			{/if}

			{#if memberPublicKey}
				<label class="cu-field">
					<span>{i18n.t.settings.identityKeyLabel}</span>
					<input
						type="text"
						readonly
						value={memberPublicKey}
						onclick={(e) => e.currentTarget.select()}
					/>
				</label>
				<button
					type="button"
					class="cu-btn"
					onclick={copyMemberPublicKey}
				>
					{i18n.t.common.copy}
				</button>
			{/if}

			{#if memberIdentityError}
				<p class="cu-error" role="alert">{memberIdentityError}</p>
			{/if}
		</section>

		{#if session.vaultInfo}
			<section class="cu-section">
				<h2>{i18n.t.settings.members}</h2>
				<p class="cu-note">
					<Rich text={i18n.t.settings.membersNote} />
				</p>

				{#if members.length > 0}
					<ul class="cu-member-list">
						{#each members as m (m.publicKey)}
							<li class="cu-member-row">
								<span class="cu-member-name">{m.name}</span>
								{#if confirmingRemoveMember === m.name}
									<div class="cu-member-confirm">
									<span class="cu-note">
										{fill(session.vaultInfo?.hasRecoveryPhrase ? i18n.t.settings.removeMemberWarningRecovery : i18n.t.settings.removeMemberWarning, { name: m.name })}
									</span>
									<input
										type="password"
										class="cu-input"
										placeholder={i18n.t.vaultForm.password}
										autocomplete="current-password"
										bind:value={removeMemberPassword}
									/>
									{#if hasUnlockMethod(unlockMethods, 'fido2')}
										<label class="cu-checkbox">
											<input
												type="checkbox"
												bind:checked={removeMemberUseFido2}
											/>
											<span>{i18n.t.settings.touchKeyToKeep}</span>
										</label>
										{#if removeMemberUseFido2}
											<input
												type="password"
												class="cu-input"
												placeholder={i18n.t.settings.keyPinIfAny}
												bind:value={removeMemberPin}
											/>
										{/if}
									{/if}
									<button
										type="button"
										class="cu-btn sm danger"
										disabled={membersBusy}
										onclick={() => removeMember(m.name)}
									>
										{i18n.t.jobs.confirmRemove}
									</button>
									<button
										type="button"
										class="cu-btn sm"
										onclick={() => (confirmingRemoveMember = null)}
									>
										{i18n.t.common.cancel}
									</button>
									</div>
								{:else}
									<button
										type="button"
										class="cu-btn sm"
										disabled={membersBusy}
										onclick={() => startRemoveMember(m.name)}
									>
										{i18n.t.common.removeEntry}
									</button>
								{/if}
							</li>
						{/each}
					</ul>
				{:else}
					<p class="cu-note">{i18n.t.settings.noMembers}</p>
				{/if}

				{#if session.pendingRemoveMemberResult}
					<div class="cu-member-result">
						<p class="cu-note">{i18n.t.settings.memberRemoved}</p>
						{#if session.pendingRemoveMemberResult.dropped.includes('fido2')}
							<p class="cu-note">
								{i18n.t.settings.fido2Invalidated}
							</p>
						{/if}
						{#if session.pendingRemoveMemberResult.dropped.includes('platform')}
							<p class="cu-note">
								{i18n.t.settings.keychainInvalidated}
							</p>
						{/if}
						{#if session.pendingRemoveMemberResult.dropped.includes('unknown')}
							<p class="cu-note">
								{i18n.t.settings.unknownMethodRemoved}
							</p>
						{/if}
						{#if session.pendingRemoveMemberResult.newRecoveryPhrase}
							<p class="cu-note">
								<Rich text={i18n.t.settings.newRecoveryPhrase} />
							</p>
							<ol class="cu-words cu-mono">
								{#each session.pendingRemoveMemberResult.newRecoveryPhrase as word, i (i)}
									<li>
										<span class="cu-idx">{i + 1}</span><span
											>{word}</span
										>
									</li>
								{/each}
							</ol>
						{/if}
						<button
							type="button"
							class="cu-btn sm"
							onclick={() => (session.pendingRemoveMemberResult = null)}
						>
							{session.pendingRemoveMemberResult.newRecoveryPhrase ? i18n.t.settings.writtenIt : i18n.t.common.ok}
						</button>
					</div>
				{/if}

				<label class="cu-field">
					<span>{i18n.t.settings.newMemberName}</span>
					<input type="text" bind:value={newMemberName} />
				</label>
				<label class="cu-field">
					<span>{i18n.t.settings.newMemberKey}</span>
					<input type="text" bind:value={newMemberPubkey} />
				</label>
				<button
					type="button"
					class="cu-btn"
					disabled={membersBusy}
					onclick={addMember}
				>
					{i18n.t.settings.addMember}
				</button>

				{#if membersError}
					<p class="cu-error" role="alert">{membersError}</p>
				{/if}
			</section>

			<section class="cu-section">
				<h2>{i18n.t.settings.inbox}</h2>
				<p class="cu-note">
					<Rich text={i18n.t.settings.inboxNote} />
				</p>

				{#if !inboxEnabled}
					<button
						type="button"
						class="cu-btn"
						disabled={inboxBusy}
						onclick={enableInbox}
					>
						{i18n.t.settings.inboxEnable}
					</button>
				{:else}
					{#if inboxPublicKey}
						<label class="cu-field">
							<span>{i18n.t.settings.inboxKeyLabel}</span>
							<input
								type="text"
								readonly
								value={inboxPublicKey}
								onclick={(e) => e.currentTarget.select()}
							/>
						</label>
						<button
							type="button"
							class="cu-btn"
							onclick={copyInboxPublicKey}
						>
							{i18n.t.common.copy}
						</button>
					{/if}

					<h3 class="cu-inbox-subheading">{i18n.t.settings.inboxPending}</h3>
					{#if inboxDeposits.length > 0}
						<ul class="cu-member-list">
							{#each inboxDeposits as d (d.depositName)}
								<li class="cu-member-row">
									<span class="cu-member-name">{d.depositName}</span>
								</li>
							{/each}
						</ul>
					{:else}
						<p class="cu-note">{i18n.t.settings.inboxNonePending}</p>
					{/if}

					<button
						type="button"
						class="cu-btn"
						disabled={inboxBusy || inboxDeposits.length === 0}
						onclick={collectInbox}
					>
						{i18n.t.settings.inboxCollect}
					</button>
					<button
						type="button"
						class="cu-btn"
						disabled={inboxBusy}
						onclick={disableInbox}
					>
						{i18n.t.settings.inboxDisable}
					</button>

					{#if inboxCollectReport}
						<div>
							{#if inboxCollectReport.collected.length > 0}
								<p class="cu-note">
									{fill(i18n.t.settings.inboxCollected, { n: inboxCollectReport.collected.length })}
									{#each inboxCollectReport.collected as c (c.vaultPath)}
										<br /><Rich text={i18n.t.settings.inboxCollectedItem} values={{ path: c.vaultPath, label: c.depositorLabel }} />
									{/each}
								</p>
							{/if}
							{#if inboxCollectReport.skipped.length > 0}
								<p class="cu-error" role="alert">
									{plural(inboxCollectReport.skipped.length, i18n.t.settings.inboxSkipped)}
									{#each inboxCollectReport.skipped as s (s.depositName)}
										<br />
										{s.depositName}: {describeMessage(s.reason)}
									{/each}
								</p>
							{/if}
						</div>
					{/if}
				{/if}

				{#if inboxError}
					<p class="cu-error" role="alert">{inboxError}</p>
				{/if}
			</section>

			<section class="cu-section">
				<h2>{i18n.t.settings.registry}</h2>
				<p class="cu-note">
					{i18n.t.settings.registryNote}
				</p>
				<div class="cu-registry-actions">
					<button
						type="button"
						class="cu-btn"
						disabled={registryBusy}
						onclick={() => runRegistry(false, false)}
					>
						{registryBusy ? i18n.t.jobs.verifyRunning : i18n.t.settings.registryVerify}
					</button>
					<button
						type="button"
						class="cu-btn"
						disabled={registryBusy}
						onclick={() => runRegistry(true, true)}
					>
						{i18n.t.settings.registrySeal}
					</button>
				</div>
				<p class="cu-note">
					{i18n.t.settings.registrySealNote}
				</p>
				{#if confirmingRegistryReset}
					<div class="cu-registry-actions">
						<p class="cu-error" role="alert">{i18n.t.settings.registryResetNote}</p>
						<button
							type="button"
							class="cu-btn danger"
							disabled={registryBusy}
							onclick={resetRegistry}
						>
							{i18n.t.settings.registryResetConfirm}
						</button>
						<button
							type="button"
							class="cu-btn"
							disabled={registryBusy}
							onclick={() => (confirmingRegistryReset = false)}
						>
							{i18n.t.common.cancel}
						</button>
					</div>
				{:else}
					<div class="cu-registry-actions">
						<button
							type="button"
							class="cu-btn"
							disabled={registryBusy}
							onclick={() => (confirmingRegistryReset = true)}
						>
							{i18n.t.settings.registryReset}
						</button>
					</div>
				{/if}
				{#if registryResetDone !== null}
					<p class="cu-note">
						{fill(i18n.t.settings.registryResetDone, { n: registryResetDone })}
					</p>
				{/if}
				{#if registryResult}
					<div>
						<p
							class={registryResult.clean ? 'cu-note' : 'cu-error'}
							role={registryResult.clean ? undefined : 'alert'}
						>
							{#if registryResult.clean}
								{fill(i18n.t.settings.registryClean, { files: registryResult.filesOk, dirs: registryResult.dirsOk })}
							{:else}
								{fill(i18n.t.settings.registryDirty, { n: registryResult.mismatches.length + registryResult.missing.length })}
							{/if}
						</p>
						{#if registryResult.unregistered > 0}
							<p class="cu-note">
								{registryResult.registered > 0 ? fill(i18n.t.settings.registryUnregisteredNow, { n: registryResult.unregistered, registered: registryResult.registered }) : fill(i18n.t.settings.registryUnregistered, { n: registryResult.unregistered })}
							</p>
						{/if}
						{#if registryResult.unresolvedPending > 0}
							<p class="cu-note">
								{fill(i18n.t.settings.registryPending, { n: registryResult.unresolvedPending })}
							</p>
						{/if}
						{#each [...registryResult.mismatches, ...registryResult.missing] as issue (issue.path)}
							<p class="cu-error">
								{issue.path}: {describeMessage(issue.problem)}
							</p>
						{/each}
						{#if registryResult.sealed}
							<p class="cu-note">
								{i18n.t.settings.registrySealed}
							</p>
						{/if}
					</div>
				{/if}
				{#if registryError}
					<p class="cu-error" role="alert">{registryError}</p>
				{/if}
			</section>

			<section class="cu-section">
				<h2>{i18n.t.recoveryKit.title}</h2>
				<p class="cu-note">
					{i18n.t.settings.kitNote}
				</p>
				<button
					type="button"
					class="cu-btn"
					disabled={recoveryKitState.busy}
					onclick={generateRecoveryKit}
				>
					{recoveryKitState.busy ? i18n.t.recovery.kitGenerating : i18n.t.settings.kitGenerate}
				</button>
				{#if recoveryKitState.error}
					<p class="cu-error" role="alert">
						{recoveryKitState.error}
					</p>
				{/if}
			</section>
		{/if}
	{/if}
</div>

<style>
	.cu-registry-actions {
		display: flex;
		gap: 10px;
		flex-wrap: wrap;
		margin-bottom: 8px;
	}

	.cu-page {
		padding: 30px 40px 26px;
		max-width: 720px;
	}

	h1 {
		margin: 0 0 24px;
	}

	.cu-section {
		margin: 0 0 28px;
	}

	.cu-section h2 {
		margin: 0 0 12px;
		font-size: 13px;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--cu-muted);
	}

	.cu-link {
		display: block;
		font-size: 12px;
		text-align: start;
		margin: 6px 0 0;
	}

	.cu-theme-row {
		display: flex;
		align-items: center;
		gap: 12px;
	}

	.cu-theme-row + .cu-theme-row {
		margin-top: 14px;
	}

	.cu-language-row {
		margin-top: 14px;
	}

	.cu-theme-row .cu-row-label {
		flex: 0 0 96px;
	}

	.cu-row-label {
		font-size: 13px;
		color: var(--cu-muted);
	}

	.cu-theme-switch {
		display: flex;
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		overflow: hidden;
	}

	.cu-theme-switch button {
		background: var(--cu-surface);
		border: none;
		color: var(--cu-text);
		padding: 6px 12px;
		font-size: 13px;
		cursor: pointer;
	}

	.cu-theme-switch button + button {
		border-inline-start: 1px solid var(--cu-line-strong);
	}

	.cu-theme-switch button.active {
		background: var(--cu-seal);
		color: var(--cu-on-seal);
	}

	.cu-field {
		display: flex;
		flex-direction: column;
		gap: 6px;
		font-size: 13px;
		margin-bottom: 8px;
	}

	.cu-strength {
		margin: 0 0 8px;
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

	.cu-inline-field {
		flex-direction: row;
		align-items: center;
		gap: 10px;
	}

	.cu-field input {
		height: 34px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		color: var(--cu-text);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		font-size: 13px;
		width: 100%;
	}

	.cu-checkbox {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 13px;
		margin-bottom: 12px;
	}

	.cu-checkbox input {
		width: auto;
	}

	.cu-note {
		color: var(--cu-muted);
		font-size: 13px;
		margin: 8px 0 0;
	}

	.cu-split-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 16px;
		margin: 8px 0 0;
	}

	.cu-split-pieces {
		margin: 8px 0 0;
		padding-inline-start: 20px;
		font-size: 12px;
	}

	.cu-split-pieces li {
		margin-bottom: 8px;
		overflow-wrap: anywhere;
	}

	.cu-unlock-method-row {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 16px;
		padding: 12px 0;
		border-top: 1px solid var(--cu-line);
	}

	.cu-unlock-method-row:first-of-type {
		border-top: none;
	}

	.cu-unlock-method-text {
		flex: 1;
		min-width: 0;
	}

	.cu-unlock-method-text strong {
		font-size: 13px;
	}

	.cu-btn {
		flex-shrink: 0;
	}

	.cu-member-list {
		list-style: none;
		margin: 0 0 16px;
		padding: 0;
	}

	.cu-member-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
		padding: 10px 0;
		border-top: 1px solid var(--cu-line);
	}

	.cu-member-row:first-child {
		border-top: none;
	}

	.cu-member-row {
		flex-wrap: wrap;
	}

	.cu-member-confirm {
		flex-basis: 100%;
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 8px;
	}

	.cu-member-confirm .cu-input {
		max-width: 320px;
	}

	.cu-member-result {
		margin: 0 0 16px;
		padding: 14px;
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-lg);
	}

	.cu-words {
		list-style: none;
		margin: 12px 0;
		padding: 14px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		display: grid;
		grid-template-columns: repeat(3, 1fr);
		gap: 6px 16px;
	}

	.cu-words li {
		font-size: 13px;
		display: flex;
		gap: 6px;
	}

	.cu-idx {
		color: var(--cu-muted);
		min-width: 1.4em;
		text-align: end;
	}

	.cu-member-name {
		font-size: 13px;
		font-weight: 500;
	}

	.cu-inbox-subheading {
		margin: 16px 0 8px;
		font-size: 13px;
		font-weight: 600;
	}

	.cu-error {
		background: color-mix(in srgb, var(--cu-conflict) 15%, transparent);
		border: 1px solid var(--cu-conflict);
		color: var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 12px 0 0;
	}
</style>
