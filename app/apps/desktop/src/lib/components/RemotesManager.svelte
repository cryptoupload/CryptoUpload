<script lang="ts">
	import Rich from './Rich.svelte';
	import { fill, i18n, plural } from '$lib/i18n/index.svelte';
	import { friendlyError, describeMessage } from '$lib/errors';
	import { open } from '@tauri-apps/plugin-dialog';
	import {
		remotesStatus,
		remotesMigrateLegacy,
		remotesFileDelete,
		remoteListNames,
		remoteUnlock,
		remoteLock,
		remoteAddS3,
		remoteAllowInsecureHttp,
		isPlainHttp,
		remoteProbeSftpHostKey,
		remoteAddSftp,
		remoteAddGdrive,
		remoteAddDropbox,
		remoteAddOneDrive,
		remoteAddWebDav,
		remoteRemove,
		remoteSweep,
		s3Presets as fetchS3Presets,
		s3PresetEndpoint,
		type Remote,
		type RemotesStatus,
		type HostKeyInfo,
		type S3PresetInfo,
		type OneDriveRootKind,
		type S3ObjectLockMode,
	} from '$lib/api/remotes';
	import { vaultCreateRemote, vaultUnlockRemote } from '$lib/api/vault';
	import PrivateProfileOption from './PrivateProfileOption.svelte';
	import { session } from '$lib/state/session.svelte';

	let status = $state<RemotesStatus | null>(null);
	let names = $state<string[]>([]);
	let error = $state<string | null>(null);

	let statusError = $state<string | null>(null);

	async function refresh() {
		statusError = null;
		try {
			status = await remotesStatus();
			names = status === 'current' ? await remoteListNames() : [];
		} catch (e) {
			statusError = friendlyError(e);
		}
	}

	$effect(() => {
		refresh();
	});

	let migratePassword = $state('');
	let migrateBusy = $state(false);
	let migrateError = $state<string | null>(null);

	async function migrate() {
		migrateError = null;
		migrateBusy = true;
		try {
			await remotesMigrateLegacy(migratePassword);
			migratePassword = '';
			await refresh();
		} catch (e) {
			migrateError = friendlyError(e);
		} finally {
			migrateBusy = false;
		}
	}

	let confirmingForgotPassword = $state(false);
	let forgotPasswordBusy = $state(false);
	let forgotPasswordError = $state<string | null>(null);

	async function forgetRemotesFile() {
		forgotPasswordError = null;
		forgotPasswordBusy = true;
		try {
			await remotesFileDelete();
			confirmingForgotPassword = false;
			migratePassword = '';
			unlockedByName = {};
			await refresh();
		} catch (e) {
			forgotPasswordError = friendlyError(e);
		} finally {
			forgotPasswordBusy = false;
		}
	}

	let unlockedByName = $state<Record<string, Remote>>({});
	let unlockingName = $state<string | null>(null);
	let unlockPassword = $state('');
	let unlockError = $state<string | null>(null);
	let unlockBusy = $state(false);

	function startUnlock(name: string) {
		unlockingName = name;
		unlockPassword = '';
		unlockError = null;
	}

	async function confirmUnlock(name: string) {
		unlockError = null;
		unlockBusy = true;
		try {
			const remote = await remoteUnlock(name, unlockPassword);
			unlockedByName = { ...unlockedByName, [name]: remote };
			unlockingName = null;
			unlockPassword = '';
		} catch (e) {
			unlockError = friendlyError(e);
		} finally {
			unlockBusy = false;
		}
	}

	async function lockRow(name: string) {
		try {
			await remoteLock(name);
		} catch (e) {
			error = friendlyError(e);
			return;
		}
		const rest = { ...unlockedByName };
		delete rest[name];
		unlockedByName = rest;
		if (vaultActionFor?.name === name) vaultActionFor = null;
	}

	const canOpenVaultHere = $derived(session.vaultInfo === null);
	let vaultActionFor = $state<{ name: string; mode: 'open' | 'create' } | null>(null);
	let vaultPassword = $state('');
	let vaultPasswordConfirm = $state('');
	let vaultPrivateProfile = $state(false);
	let vaultDecoyFiles = $state(false);
	let vaultActionError = $state<string | null>(null);
	let vaultActionBusy = $state(false);

	async function confirmVaultAction() {
		if (!vaultActionFor) return;
		vaultActionError = null;
		if (vaultActionFor.mode === 'create' && vaultPassword !== vaultPasswordConfirm) {
			vaultActionError = i18n.t.vaultForm.passwordsDiffer;
			return;
		}
		vaultActionBusy = true;
		try {
			const info =
				vaultActionFor.mode === 'create'
					? await vaultCreateRemote(vaultActionFor.name, vaultPassword, vaultPrivateProfile, vaultDecoyFiles)
					: await vaultUnlockRemote(vaultActionFor.name, vaultPassword);
			session.vaultRoot = `remote:${vaultActionFor.name}`;  // i18n-ignore
			session.vaultInfo = info;
			session.screen = vaultActionFor.mode === 'create' ? 'recovery-reveal' : 'unlocked';
			vaultActionFor = null;
			vaultPassword = '';
			vaultPasswordConfirm = '';
		} catch (e) {
			vaultActionError = friendlyError(e);
		} finally {
			vaultActionBusy = false;
		}
	}

	let confirmingRemove = $state<string | null>(null);
	let removePassword = $state('');
	let removeError = $state<string | null>(null);
	let removeBusy = $state(false);

	function startRemove(name: string) {
		confirmingRemove = name;
		removePassword = '';
		removeError = null;
	}

	async function confirmRemove(name: string) {
		removeError = null;
		removeBusy = true;
		try {
			await remoteRemove(name, removePassword);
			confirmingRemove = null;
			removePassword = '';
			await lockRow(name);
			await refresh();
		} catch (e) {
			removeError = friendlyError(e);
		} finally {
			removeBusy = false;
		}
	}

	let sweepResult = $state<{ name: string; text: string } | null>(null);
	const SWEEP_MIN_AGE_DAYS = 7;
	let confirmingSweep = $state<string | null>(null);

	async function sweep(name: string) {
		confirmingSweep = null;
		error = null;
		try {
			const cleaned = await remoteSweep(name, SWEEP_MIN_AGE_DAYS);
			sweepResult = {
				name,
				text:
					cleaned.length === 0
						? i18n.t.remotes.sweepNone
						: plural(cleaned.length, i18n.t.remotes.sweepDone),
			};
		} catch (e) {
			error = friendlyError(e);
		}
	}

	let showAdd = $state(false);
	let provider = $state<'s3' | 'sftp' | 'gdrive' | 'dropbox' | 'onedrive' | 'webdav'>('s3');
	let newName = $state('');
	let newPassword = $state('');
	let newPasswordConfirm = $state('');

	let s3PresetId = $state('custom');
	let s3PresetOptions = $state<S3PresetInfo[]>([]);
	let s3Endpoint = $state('');
	let s3Region = $state('us-east-1');
	let s3Bucket = $state('');
	let s3Prefix = $state('');
	let s3PathStyle = $state(true);
	let s3AccessKey = $state('');
	let s3SecretKey = $state('');
	let s3ObjectLockEnabled = $state(false);
	let s3ObjectLockMode = $state<S3ObjectLockMode>('governance');
	let s3ObjectLockDays = $state(30);
	let s3AllowInsecureHttp = $state(false);

	$effect(() => {
		fetchS3Presets().then((p) => (s3PresetOptions = p));
	});

	function currentS3Preset(): S3PresetInfo | undefined {
		return s3PresetOptions.find((p) => p.id === s3PresetId);
	}

	async function applyS3Preset(id: string) {
		s3PresetId = id;
		const preset = s3PresetOptions.find((p) => p.id === id);
		if (!preset) return;
		s3PathStyle = preset.pathStyle;
		if (preset.regions.length > 0) {
			s3Region = preset.regions[0];
		}
		if (preset.hasEndpoint) {
			const endpoint = await s3PresetEndpoint(id, s3Region);
			if (endpoint) s3Endpoint = endpoint;
		}
	}

	async function applyS3Region(region: string) {
		s3Region = region;
		const preset = currentS3Preset();
		if (preset?.hasEndpoint) {
			const endpoint = await s3PresetEndpoint(s3PresetId, region);
			if (endpoint) s3Endpoint = endpoint;
		}
	}

	let sftpHost = $state('');
	let sftpPort = $state(22);
	let sftpUsername = $state('');
	let sftpAuthMethod = $state<'password' | 'key'>('password');
	let sftpPassword = $state('');
	let sftpKeyPath = $state('');
	let sftpKeyPassphrase = $state('');
	let sftpRemoteRoot = $state('');
	let sftpHostKey = $state<HostKeyInfo | null>(null);
	let sftpHostKeyError = $state<string | null>(null);
	let sftpProbing = $state(false);

	let gdriveClientId = $state('');
	let gdriveClientSecret = $state('');
	// i18n-ignore
	let gdriveFolderName = $state('CryptoUpload Vault');
	let gdriveWaiting = $state(false);

	let dropboxClientId = $state('');
	let dropboxWaiting = $state(false);

	let onedriveClientId = $state('');
	let onedriveClientSecret = $state('');
	let onedriveTenant = $state('common');
	let onedriveRootKind = $state<OneDriveRootKind>('app_folder');
	let onedriveWaiting = $state(false);

	let webdavBaseUrl = $state('');
	let webdavUsername = $state('');
	let webdavPassword = $state('');
	let webdavRootPath = $state('');
	let webdavAllowInsecureHttp = $state(false);

	function resetAddForm() {
		newName = '';
		newPassword = '';
		newPasswordConfirm = '';
		s3PresetId = 'custom';
		s3Endpoint = '';
		s3Bucket = '';
		s3Prefix = '';
		s3AccessKey = '';
		s3SecretKey = '';
		s3ObjectLockEnabled = false;
		s3AllowInsecureHttp = false;
		s3ObjectLockMode = 'governance';
		s3ObjectLockDays = 30;
		sftpHost = '';
		sftpUsername = '';
		sftpPassword = '';
		sftpKeyPath = '';
		sftpKeyPassphrase = '';
		sftpRemoteRoot = '';
		sftpHostKey = null;
		gdriveClientId = '';
		gdriveClientSecret = '';
		dropboxClientId = '';
		onedriveClientId = '';
		onedriveClientSecret = '';
		onedriveTenant = 'common';
		onedriveRootKind = 'app_folder';
		webdavBaseUrl = '';
		webdavUsername = '';
		webdavPassword = '';
		webdavRootPath = '';
		webdavAllowInsecureHttp = false;
	}

	function newPasswordOk(): boolean {
		if (newPassword !== newPasswordConfirm) {
			error = i18n.t.vaultForm.passwordsDiffer;
			return false;
		}
		if (newPassword.length === 0) {
			error = i18n.t.remotes.passwordEmpty;
			return false;
		}
		return true;
	}

	async function addS3() {
		error = null;
		if (!newPasswordOk()) return;
		try {
			await remoteAddS3(
				newName.trim(),
				s3Endpoint,
				s3Region,
				s3Bucket,
				s3Prefix,
				s3PathStyle,
				s3AccessKey,
				s3SecretKey,
				newPassword,
				s3ObjectLockEnabled ? s3ObjectLockMode : null,
				s3ObjectLockEnabled ? s3ObjectLockDays : null,
				isPlainHttp(s3Endpoint) && s3AllowInsecureHttp,
			);
			showAdd = false;
			resetAddForm();
			await refresh();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function allowS3Http(name: string) {
		error = null;
		try {
			const remote = await remoteAllowInsecureHttp(name);
			unlockedByName = { ...unlockedByName, [name]: remote };
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function chooseSftpKeyPath() {
		const picked = await open({ directory: false, multiple: false });
		if (typeof picked === 'string') {
			sftpKeyPath = picked;
		}
	}

	async function probeSftpHostKey() {
		sftpHostKeyError = null;
		sftpProbing = true;
		try {
			sftpHostKey = await remoteProbeSftpHostKey(sftpHost, sftpPort);
		} catch (e) {
			sftpHostKeyError = friendlyError(e);
		} finally {
			sftpProbing = false;
		}
	}

	async function addSftp() {
		if (!sftpHostKey) return;
		error = null;
		if (!newPasswordOk()) return;
		try {
			await remoteAddSftp(
				newName.trim(),
				sftpHost,
				sftpPort,
				sftpUsername,
				sftpAuthMethod,
				sftpAuthMethod === 'password' ? sftpPassword : null,
				sftpAuthMethod === 'key' ? sftpKeyPath : null,
				sftpAuthMethod === 'key' && sftpKeyPassphrase ? sftpKeyPassphrase : null,
				sftpRemoteRoot,
				sftpHostKey.keyType,
				sftpHostKey.fingerprint,
				newPassword,
			);
			showAdd = false;
			resetAddForm();
			await refresh();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function addGdrive() {
		error = null;
		if (!newPasswordOk()) return;
		gdriveWaiting = true;
		try {
			await remoteAddGdrive(
				newName.trim(),
				gdriveClientId,
				gdriveClientSecret,
				gdriveFolderName,
				newPassword,
			);
			showAdd = false;
			resetAddForm();
			await refresh();
		} catch (e) {
			error = friendlyError(e);
		} finally {
			gdriveWaiting = false;
		}
	}

	async function addDropbox() {
		error = null;
		if (!newPasswordOk()) return;
		dropboxWaiting = true;
		try {
			await remoteAddDropbox(newName.trim(), dropboxClientId, newPassword);
			showAdd = false;
			resetAddForm();
			await refresh();
		} catch (e) {
			error = friendlyError(e);
		} finally {
			dropboxWaiting = false;
		}
	}

	async function addOneDrive() {
		error = null;
		if (!newPasswordOk()) return;
		onedriveWaiting = true;
		try {
			await remoteAddOneDrive(
				newName.trim(),
				onedriveClientId,
				onedriveClientSecret.length > 0 ? onedriveClientSecret : null,
				onedriveTenant.trim().length > 0 ? onedriveTenant.trim() : 'common',
				onedriveRootKind,
				newPassword,
			);
			showAdd = false;
			resetAddForm();
			await refresh();
		} catch (e) {
			error = friendlyError(e);
		} finally {
			onedriveWaiting = false;
		}
	}

	async function addWebDav() {
		error = null;
		if (!newPasswordOk()) return;
		try {
			await remoteAddWebDav(
				newName.trim(),
				webdavBaseUrl,
				webdavUsername,
				webdavPassword,
				webdavRootPath,
				webdavAllowInsecureHttp,
				newPassword,
			);
			showAdd = false;
			resetAddForm();
			await refresh();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	function remoteSummary(r: Remote): string {
		switch (r.provider) {
			case 's3': {
				const scheme = r.allowInsecureHttp ? ` ${i18n.t.remotes.httpActive}` : '';
				return fill(i18n.t.remotes.summaryS3, { endpoint: r.endpoint, bucket: r.bucket }) + scheme;
			}
			case 'sftp':
				return `sftp — ${r.username}@${r.host}:${r.port}`;
			case 'gdrive':
				return fill(i18n.t.remotes.summaryGdrive, { folder: r.folderId });
			case 'dropbox':
				return fill(i18n.t.remotes.summaryDropbox, { client: r.clientId });
			case 'onedrive': {
				const root = r.rootKind === 'drive_root' ? i18n.t.remotes.rootWholeDrive : i18n.t.remotes.rootAppFolder;
				return fill(i18n.t.remotes.summaryOnedrive, { client: r.clientId, tenant: r.tenant, root });
			}
			case 'webdav': {
				const scheme = r.allowInsecureHttp ? ` ${i18n.t.remotes.httpActive}` : '';
				return `webdav — ${r.username}@${r.baseUrl}${scheme}`;
			}
			default:
				return r satisfies never;
		}
	}
</script>

<div class="cu-page">
	<h1 class="cu-page-title">{i18n.t.nav.remotes}</h1>

	{#if error}<p class="cu-error" role="alert">{error}</p>{/if}

	{#if statusError}
		<p class="cu-error" role="alert">
			{fill(i18n.t.remotes.listFailed, { error: statusError })}
		</p>
		<button type="button" class="cu-btn" onclick={refresh}>{i18n.t.common.retry}</button>
	{:else if status === null}
		<p class="cu-note">{i18n.t.common.verifying}</p>
	{:else if status === 'legacy'}
		<div class="cu-card">
			<p class="cu-note">
				{i18n.t.remotes.legacyNote}
			</p>
			<form
				onsubmit={(e) => {
					e.preventDefault();
					migrate();
				}}
			>
				<label class="cu-field">
					<span>{i18n.t.remotes.legacyPassword}</span>
					<input
						type="password"
						bind:value={migratePassword}
						autocomplete="current-password"
					/>
				</label>
				{#if migrateError}<p class="cu-error" role="alert">{migrateError}</p>{/if}
				<button
					type="submit"
					class="cu-btn primary"
					disabled={migrateBusy}
				>
					{i18n.t.common.update}
				</button>
			</form>

			{#if confirmingForgotPassword}
				<p class="cu-note">
					<Rich text={i18n.t.remotes.forgotWarning} />
				</p>
				{#if forgotPasswordError}<p class="cu-error" role="alert">{forgotPasswordError}</p>{/if}
				<button
					class="cu-btn danger"
					disabled={forgotPasswordBusy}
					onclick={forgetRemotesFile}
				>
					{i18n.t.remotes.forgotConfirm}
				</button>
				<button type="button" class="cu-btn" onclick={() => (confirmingForgotPassword = false)}>{i18n.t.common.cancel}</button>
			{:else}
				<button
					class="cu-link-forgot cu-text-link"
					onclick={() => (confirmingForgotPassword = true)}
				>
					{i18n.t.remotes.forgotLegacyLink}
				</button>
			{/if}
		</div>
	{:else}
		{#if names.length === 0}
			<p class="cu-note">{i18n.t.remotes.empty}</p>
		{:else}
			<ul class="cu-remotes">
				{#each names as name (name)}
					{@const unlocked = unlockedByName[name]}
					<li>
						<div class="cu-remote-row">
							<div>
								<div class="cu-remote-name">
									{#if unlocked}
										<svg class="cu-i cu-remote-state cu-remote-state-ok"><use href="#cu-i-check" /></svg>
									{:else}
										<svg class="cu-i cu-remote-state"><use href="#cu-i-lock" /></svg>
									{/if}
									{name}
								</div>
								{#if unlocked}
									<div class="cu-remote-detail cu-mono">
										{remoteSummary(unlocked)}
									</div>
									{#if unlocked.provider === 's3' && isPlainHttp(unlocked.endpoint) && !unlocked.allowInsecureHttp}
										<p class="cu-warning">
											<svg class="cu-i"><use href="#cu-i-warn" /></svg>
											<span>{describeMessage('S3_HTTP_WARNING')}</span>
										</p>
										<button type="button" class="cu-btn sm" onclick={() => allowS3Http(name)}>
											{i18n.t.remotes.allowHttp}
										</button>
									{/if}
								{/if}
							</div>
							<div class="cu-remote-actions">
								{#if unlocked}
									<button type="button" class="cu-btn sm" onclick={() => lockRow(name)}>{i18n.t.remotes.lock}</button>
								{:else}
									<button type="button" class="cu-btn sm" onclick={() => startUnlock(name)}>
										{i18n.t.remotes.select}
									</button>
								{/if}
								{#if unlocked && canOpenVaultHere}
									<button type="button" class="cu-btn sm" onclick={() => (vaultActionFor = { name, mode: 'open' })}>
										{i18n.t.remotes.unlockVaultHere}
									</button>
									<button type="button" class="cu-btn sm" onclick={() => (vaultActionFor = { name, mode: 'create' })}>
										{i18n.t.remotes.createVaultHere}
									</button>
								{/if}
								{#if unlocked && unlocked.provider === 's3'}
									{#if confirmingSweep === name}
										<span class="cu-note">
											{fill(i18n.t.remotes.sweepNote, { days: SWEEP_MIN_AGE_DAYS })}
										</span>
										<button
											class="cu-btn sm danger"
											onclick={() => sweep(name)}
										>
											{i18n.t.remotes.sweepConfirm}
										</button>
										<button type="button" class="cu-btn sm" onclick={() => (confirmingSweep = null)}>{i18n.t.common.cancel}</button>
									{:else}
										<button
											type="button"
											class="cu-btn sm"
											onclick={() => (confirmingSweep = name)}
										>
											{i18n.t.remotes.sweep}
										</button>
									{/if}
								{/if}
								{#if confirmingRemove === name}
									<button
										class="cu-btn sm danger"
										onclick={() => confirmRemove(name)}
										disabled={removeBusy}
									>
										{i18n.t.remotes.removeForGood}
									</button>
									<button type="button" class="cu-btn sm" onclick={() => (confirmingRemove = null)}>{i18n.t.common.cancel}</button>
								{:else}
									<button type="button" class="cu-btn sm" onclick={() => startRemove(name)}>
										{i18n.t.common.removeEntry}
									</button>
								{/if}
							</div>
						</div>

						{#if unlockingName === name}
							<form
								class="cu-vault-action"
								onsubmit={(e) => {
									e.preventDefault();
									confirmUnlock(name);
								}}
							>
								<input
									type="password"
									bind:value={unlockPassword}
									placeholder={i18n.t.remotes.remotePasswordPlaceholder}
									autocomplete="current-password"
								/>
								<button
									type="submit"
									class="cu-btn primary"
									disabled={unlockBusy}
								>
									{i18n.t.unlock.submitUnlock}
								</button>
								<button type="button" class="cu-btn" onclick={() => (unlockingName = null)}>{i18n.t.common.cancel}</button>
							</form>
							{#if unlockError}<p class="cu-error" role="alert">{unlockError}</p>{/if}
						{/if}

						{#if confirmingRemove === name}
							<p class="cu-note">
								<Rich text={i18n.t.remotes.removeNote} values={{ name }} />
							</p>
							<form
								onsubmit={(e) => {
									e.preventDefault();
									confirmRemove(name);
								}}
							>
								<input
									type="password"
									bind:value={removePassword}
									placeholder={i18n.t.remotes.remotePasswordPlaceholder}
									autocomplete="current-password"
								/>
							</form>
							{#if removeError}<p class="cu-error" role="alert">{removeError}</p>{/if}
						{/if}

						{#if sweepResult?.name === name}
							<p class="cu-note">{sweepResult.text}</p>
						{/if}

						{#if vaultActionFor?.name === name}
							<form
								class="cu-vault-action"
								onsubmit={(e) => {
									e.preventDefault();
									confirmVaultAction();
								}}
							>
								<input
									type="password"
									bind:value={vaultPassword}
									placeholder={i18n.t.jobs.vaultPasswordPlaceholder}
									autocomplete={vaultActionFor.mode === 'create' ? 'new-password' : 'current-password'}
								/>
								{#if vaultActionFor.mode === 'create'}
									<input
										type="password"
										bind:value={vaultPasswordConfirm}
										placeholder={i18n.t.remotes.repeatPasswordPlaceholder}
										autocomplete="new-password"
									/>
									<PrivateProfileOption bind:checked={vaultPrivateProfile} bind:decoys={vaultDecoyFiles} />
								{/if}
								<button type="submit" class="cu-btn primary" disabled={vaultActionBusy}>
									{vaultActionFor.mode === 'create' ? i18n.t.common.create : i18n.t.unlock.submitUnlock}
								</button>
								<button type="button" class="cu-btn" onclick={() => (vaultActionFor = null)}>{i18n.t.common.cancel}</button>
							</form>
							{#if vaultActionError}<p class="cu-error" role="alert">{vaultActionError}</p>{/if}
						{/if}
					</li>
				{/each}
			</ul>
		{/if}

		<button class="cu-btn primary" onclick={() => (showAdd = !showAdd)}>
			{i18n.t.remotes.add}
		</button>

		{#if showAdd}
			<div class="cu-card">
				<label class="cu-field">
					<span>{i18n.t.jobs.name}</span>
					<input type="text" bind:value={newName} placeholder={i18n.t.remotes.namePlaceholder} />
				</label>
				<div class="cu-provider-tabs">
					<button
						class:active={provider === 's3'}
						onclick={() => (provider = 's3')}
					>
						S3
					</button>
					<button
						class:active={provider === 'sftp'}
						onclick={() => (provider = 'sftp')}
					>
						SFTP
					</button>
					<button
						class:active={provider === 'gdrive'}
						onclick={() => (provider = 'gdrive')}
					>
						Google Drive
					</button>
					<button
						class:active={provider === 'dropbox'}
						onclick={() => (provider = 'dropbox')}
					>
						Dropbox
					</button>
					<button
						class:active={provider === 'onedrive'}
						onclick={() => (provider = 'onedrive')}
					>
						OneDrive
					</button>
					<button
						class:active={provider === 'webdav'}
						onclick={() => (provider = 'webdav')}
					>
						WebDAV
					</button>
				</div>

				{#if provider === 's3'}
					<form
						onsubmit={(e) => {
							e.preventDefault();
							addS3();
						}}
					>
						<label class="cu-field">
							<span>{i18n.t.remotes.preset}</span>
							<select
								value={s3PresetId}
								onchange={(e) => applyS3Preset((e.target as HTMLSelectElement).value)}
							>
								{#each s3PresetOptions as preset (preset.id)}
									<option value={preset.id}>{preset.label}</option>
								{/each}
							</select>
						</label>
						{#if currentS3Preset()?.note}
							<p class="cu-note">{describeMessage(currentS3Preset()?.note ?? '')}</p>
						{/if}
						<label class="cu-field">
							<span>{i18n.t.remotes.endpoint}</span>
							<input
								class="cu-mono"
								type="text"
								bind:value={s3Endpoint}
								placeholder="http://127.0.0.1:9000"
							/>
						</label>
						{#if (currentS3Preset()?.regions.length ?? 0) > 0}
							<label class="cu-field">
								<span>{i18n.t.remotes.region}</span>
								<select
									value={s3Region}
									onchange={(e) => applyS3Region((e.target as HTMLSelectElement).value)}
								>
									{#each currentS3Preset()?.regions ?? [] as region (region)}
										<option value={region}>{region}</option>
									{/each}
								</select>
							</label>
						{:else}
							<label class="cu-field">
								<span>{i18n.t.remotes.region}</span>
								<input class="cu-mono" type="text" bind:value={s3Region} />
							</label>
						{/if}
						<label class="cu-field">
							<span>{i18n.t.remotes.bucket}</span>
							<input class="cu-mono" type="text" bind:value={s3Bucket} />
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.prefix}</span>
							<input class="cu-mono" type="text" bind:value={s3Prefix} />
						</label>
						<label class="cu-checkbox">
							<input type="checkbox" bind:checked={s3PathStyle} />
							<span>{i18n.t.remotes.pathStyle}</span>
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.accessKey}</span>
							<input class="cu-mono" type="text" bind:value={s3AccessKey} />
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.secretKey}</span>
							<input type="password" bind:value={s3SecretKey} />
						</label>
						<label class="cu-checkbox">
							<input
								type="checkbox"
								bind:checked={s3ObjectLockEnabled}
							/>
							<span>{i18n.t.remotes.objectLock}</span>
						</label>
						{#if s3ObjectLockEnabled}
							<p class="cu-warning">
								<svg class="cu-i"><use href="#cu-i-warn" /></svg>
								<span>
								<Rich text={i18n.t.remotes.objectLockNote} />
								</span>
							</p>
							<label class="cu-field">
								<span>{i18n.t.remotes.mode}</span>
								<select
									bind:value={s3ObjectLockMode}
								>
									<option value="governance"
										>{i18n.t.remotes.modeGovernance}</option
									>
									<option value="compliance">{i18n.t.remotes.modeCompliance}</option
									>
								</select>
							</label>
							<label class="cu-field">
								<span>{i18n.t.remotes.retentionDays}</span>
								<input
									class="cu-mono"
									type="number"
									min="1"
									bind:value={s3ObjectLockDays}
								/>
							</label>
						{/if}
						{#if isPlainHttp(s3Endpoint)}
							<label class="cu-checkbox">
								<input type="checkbox" bind:checked={s3AllowInsecureHttp} />
								<span>{i18n.t.remotes.allowHttp}</span>
							</label>
							{#if s3AllowInsecureHttp}
								<p class="cu-warning">
									<svg class="cu-i"><use href="#cu-i-warn" /></svg>
									<span>{describeMessage('S3_HTTP_WARNING')}</span>
								</p>
							{/if}
						{/if}
						<label class="cu-field">
							<span>{i18n.t.remotes.entryPassword}</span>
							<input
								type="password"
								bind:value={newPassword}
								autocomplete="new-password"
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.vaultForm.repeatPassword}</span>
							<input
								type="password"
								bind:value={newPasswordConfirm}
								autocomplete="new-password"
							/>
						</label>
						<button type="submit" class="cu-btn primary">
							{i18n.t.remotes.saveS3}
						</button>
					</form>
				{:else if provider === 'sftp'}
					<form
						onsubmit={(e) => {
							e.preventDefault();
							if (sftpHostKey) addSftp();
							else probeSftpHostKey();
						}}
					>
						<label class="cu-field">
							<span>{i18n.t.remotes.host}</span>
							<input class="cu-mono" type="text" bind:value={sftpHost} />
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.port}</span>
							<input type="number" bind:value={sftpPort} />
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.user}</span>
							<input class="cu-mono" type="text" bind:value={sftpUsername} />
						</label>
						<div class="cu-provider-tabs">
							<button
								type="button"
								class:active={sftpAuthMethod === 'password'}
								onclick={() => (sftpAuthMethod = 'password')}
							>
								{i18n.t.remotes.passwordPlain}
							</button>
							<button
								type="button"
								class:active={sftpAuthMethod === 'key'}
								onclick={() => (sftpAuthMethod = 'key')}
							>
								{i18n.t.remotes.privateKey}
							</button>
						</div>
						{#if sftpAuthMethod === 'password'}
							<label class="cu-field">
								<span>{i18n.t.remotes.sshPassword}</span>
								<input type="password" bind:value={sftpPassword} />
							</label>
						{:else}
							<label class="cu-field">
								<span>{i18n.t.remotes.privateKeyPath}</span>
								<div class="cu-path-row">
									<input class="cu-mono" type="text" bind:value={sftpKeyPath} placeholder={'~/.ssh/id_ed25519'} />
									<button type="button" class="cu-btn" onclick={chooseSftpKeyPath}>{i18n.t.common.choose}</button>
								</div>
							</label>
							<label class="cu-field">
								<span>{i18n.t.remotes.keyPassphrase}</span>
								<input type="password" bind:value={sftpKeyPassphrase} />
							</label>
						{/if}
						<label class="cu-field">
							<span>{i18n.t.remotes.remoteRoot}</span>
							<input class="cu-mono" type="text" bind:value={sftpRemoteRoot} />
						</label>

						{#if sftpHostKeyError}<p class="cu-error" role="alert">{sftpHostKeyError}</p>{/if}

						{#if sftpHostKey}
							<div class="cu-hostkey">
								<p>
									<Rich text={i18n.t.remotes.hostKeySeen} values={{ type: sftpHostKey.keyType, fingerprint: sftpHostKey.fingerprint }} />
								</p>
								<p class="cu-note">{i18n.t.remotes.hostKeyNote}</p>
							</div>
							<label class="cu-field">
								<span>{i18n.t.remotes.entryPassword}</span>
								<input type="password" bind:value={newPassword} autocomplete="new-password" />
							</label>
							<label class="cu-field">
								<span>{i18n.t.vaultForm.repeatPassword}</span>
								<input type="password" bind:value={newPasswordConfirm} autocomplete="new-password" />
							</label>
							<button type="submit" class="cu-btn primary">{i18n.t.remotes.trustSaveSftp}</button>
						{:else}
							<button type="submit" class="cu-btn primary" disabled={sftpProbing}>
								{sftpProbing ? i18n.t.common.verifying : i18n.t.remotes.verifyHostKey}
							</button>
						{/if}
					</form>
				{:else if provider === 'gdrive'}
					<form
						onsubmit={(e) => {
							e.preventDefault();
							addGdrive();
						}}
					>
						<p class="cu-note">
							<Rich text={i18n.t.remotes.gdriveNote} />
						</p>
						<label class="cu-field">
							<span>{i18n.t.remotes.clientId}</span>
							<input class="cu-mono" type="text" bind:value={gdriveClientId} />
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.clientSecret}</span>
							<input type="password" bind:value={gdriveClientSecret} />
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.gdriveFolder}</span>
							<input class="cu-mono" type="text" bind:value={gdriveFolderName} />
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.entryPassword}</span>
							<input type="password" bind:value={newPassword} autocomplete="new-password" />
						</label>
						<label class="cu-field">
							<span>{i18n.t.vaultForm.repeatPassword}</span>
							<input type="password" bind:value={newPasswordConfirm} autocomplete="new-password" />
						</label>
						<button type="submit" class="cu-btn primary" disabled={gdriveWaiting}>
							{gdriveWaiting ? i18n.t.remotes.waitingConsent : i18n.t.remotes.openBrowser}
						</button>
					</form>
				{:else if provider === 'dropbox'}
					<form
						onsubmit={(e) => {
							e.preventDefault();
							addDropbox();
						}}
					>
						<p class="cu-note">
							{i18n.t.remotes.dropboxNote}
						</p>
						<label class="cu-field">
							<span>{i18n.t.remotes.clientId}</span>
							<input
								class="cu-mono"
								type="text"
								bind:value={dropboxClientId}
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.entryPassword}</span>
							<input
								type="password"
								bind:value={newPassword}
								autocomplete="new-password"
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.vaultForm.repeatPassword}</span>
							<input
								type="password"
								bind:value={newPasswordConfirm}
								autocomplete="new-password"
							/>
						</label>
						<button
							type="submit"
							class="cu-btn primary"
							disabled={dropboxWaiting}
						>
							{dropboxWaiting ? i18n.t.remotes.waitingConsent : i18n.t.remotes.openBrowser}
						</button>
					</form>
				{:else if provider === 'onedrive'}
					<form
						onsubmit={(e) => {
							e.preventDefault();
							addOneDrive();
						}}
					>
						<p class="cu-note">
							{i18n.t.remotes.onedriveNote}
						</p>
						<label class="cu-field">
							<span>{i18n.t.remotes.clientId}</span>
							<input
								class="cu-mono"
								type="text"
								bind:value={onedriveClientId}
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.clientSecretAdvanced}</span>
							<input
								type="password"
								bind:value={onedriveClientSecret}
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.tenant}</span>
							<input
								class="cu-mono"
								type="text"
								bind:value={onedriveTenant}
								placeholder={'common'}
							/>
						</label>
						<fieldset class="cu-field">
							<legend>{i18n.t.remotes.vaultRoot}</legend>
							<label class="cu-radio">
								<input
									type="radio"
									name="onedrive-root-kind"
									value="app_folder"
									checked={onedriveRootKind === 'app_folder'}
									onchange={() => (onedriveRootKind = 'app_folder')}
								/>
								<span>
									{i18n.t.remotes.rootAppFolderLong}
								</span>
							</label>
							<label class="cu-radio">
								<input
									type="radio"
									name="onedrive-root-kind"
									value="drive_root"
									checked={onedriveRootKind === 'drive_root'}
									onchange={() => (onedriveRootKind = 'drive_root')}
								/>
								<span>
									{i18n.t.remotes.rootWholeDriveLong}
								</span>
							</label>
						</fieldset>
						<p class="cu-note">
							{fill(i18n.t.remotes.rootChosen, { root: onedriveRootKind === 'drive_root' ? i18n.t.remotes.rootWholeDrive : i18n.t.remotes.rootAppFolderOf })}
						</p>
						<label class="cu-field">
							<span>{i18n.t.remotes.entryPassword}</span>
							<input
								type="password"
								bind:value={newPassword}
								autocomplete="new-password"
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.vaultForm.repeatPassword}</span>
							<input
								type="password"
								bind:value={newPasswordConfirm}
								autocomplete="new-password"
							/>
						</label>
						<button
							type="submit"
							class="cu-btn primary"
							disabled={onedriveWaiting}
						>
							{onedriveWaiting ? i18n.t.remotes.waitingConsent : i18n.t.remotes.openBrowser}
						</button>
					</form>
				{:else}
					<form
						onsubmit={(e) => {
							e.preventDefault();
							addWebDav();
						}}
					>
						<p class="cu-note">
							{i18n.t.remotes.webdavNote}
						</p>
						<label class="cu-field">
							<span>{i18n.t.remotes.baseUrl}</span>
							<input
								class="cu-mono"
								type="text"
								bind:value={webdavBaseUrl}
								placeholder="https://cloud.example.org/remote.php/dav/files/username"
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.user}</span>
							<input
								class="cu-mono"
								type="text"
								bind:value={webdavUsername}
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.passwordOrToken}</span>
							<input
								type="password"
								bind:value={webdavPassword}
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.remotes.webdavPrefix}</span>
							<input class="cu-mono" type="text" bind:value={webdavRootPath} />
						</label>
						<label class="cu-checkbox">
							<input
								type="checkbox"
								bind:checked={webdavAllowInsecureHttp}
							/>
							<span>{i18n.t.remotes.allowHttp}</span>
						</label>
						{#if webdavAllowInsecureHttp}
							<p class="cu-warning">
								<svg class="cu-i"><use href="#cu-i-warn" /></svg>
								<span>
								<Rich text={i18n.t.remotes.allowHttpWarning} />
								</span>
							</p>
						{/if}
						<label class="cu-field">
							<span>{i18n.t.remotes.entryPassword}</span>
							<input
								type="password"
								bind:value={newPassword}
								autocomplete="new-password"
							/>
						</label>
						<label class="cu-field">
							<span>{i18n.t.vaultForm.repeatPassword}</span>
							<input
								type="password"
								bind:value={newPasswordConfirm}
								autocomplete="new-password"
							/>
						</label>
						<button type="submit" class="cu-btn primary">
							{i18n.t.remotes.saveWebdav}
						</button>
					</form>
				{/if}
			</div>
		{/if}

		{#if confirmingForgotPassword}
			<p class="cu-note">
				<Rich text={i18n.t.remotes.forgotWarningLong} />
			</p>
			{#if forgotPasswordError}<p class="cu-error" role="alert">{forgotPasswordError}</p>{/if}
			<button
				class="cu-btn danger"
				disabled={forgotPasswordBusy}
				onclick={forgetRemotesFile}
			>
				{i18n.t.remotes.forgotConfirm}
			</button>
			<button type="button" class="cu-btn" onclick={() => (confirmingForgotPassword = false)}>{i18n.t.common.cancel}</button>
		{:else}
			<button
				class="cu-link-forgot cu-text-link"
				onclick={() => (confirmingForgotPassword = true)}
			>
				{i18n.t.remotes.forgotLink}
			</button>
		{/if}
	{/if}
</div>

<style>
	.cu-page {
		padding: 30px 40px 26px;
		max-width: 720px;
	}

	h1 {
		margin: 0 0 16px;
	}

	.cu-card {
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		padding: 16px;
		margin: 12px 0 20px;
	}

	.cu-field {
		display: block;
		margin-bottom: 12px;
	}

	.cu-field span {
		display: block;
		font-size: 12px;
		color: var(--cu-muted);
		margin-bottom: 4px;
	}

	.cu-field input,
	.cu-vault-action input {
		width: 100%;
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		color: var(--cu-text);
	}

	.cu-path-row {
		display: flex;
		gap: 8px;
	}

	.cu-path-row input {
		flex: 1;
		min-width: 0;
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
		height: auto;
	}

	fieldset.cu-field {
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-sm);
		padding: 10px 12px 4px;
	}

	fieldset.cu-field legend {
		font-size: 12px;
		color: var(--cu-muted);
		padding: 0 4px;
	}

	.cu-radio {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		font-size: 13px;
		margin-bottom: 10px;
	}

	.cu-radio input {
		width: auto;
		height: auto;
		margin-top: 3px;
	}

	.cu-provider-tabs {
		display: flex;
		gap: 8px;
		margin-bottom: 14px;
	}

	.cu-provider-tabs button {
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		color: var(--cu-muted);
		border-radius: var(--cu-radius-sm);
		padding: 6px 12px;
		font-size: 13px;
		cursor: pointer;
	}

	.cu-provider-tabs button.active {
		color: var(--cu-text);
		border-color: var(--cu-seal-text);
	}

	.cu-error {
		background: color-mix(in srgb, var(--cu-conflict) 15%, transparent);
		border: 1px solid var(--cu-conflict);
		color: var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 10px 0;
	}

	.cu-note {
		color: var(--cu-muted);
		font-size: 13px;
	}

	.cu-warning {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		background: color-mix(in srgb, var(--cu-conflict) 12%, transparent);
		border: 1px solid var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 6px 0 10px;
	}

	.cu-warning .cu-i {
		flex-shrink: 0;
		margin-top: 1px;
		color: var(--cu-conflict);
	}

	.cu-link-forgot {
		display: block;
		font-size: 12px;
		text-align: start;
		margin-top: 12px;
	}

	.cu-hostkey {
		background: var(--cu-bg);
		border: 1px solid var(--cu-seal);
		border-radius: var(--cu-radius-sm);
		padding: 10px 12px;
		font-size: 13px;
		margin-bottom: 12px;
	}

	.cu-remotes {
		list-style: none;
		margin: 16px 0 0;
		padding: 0;
	}

	.cu-remotes > li {
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		padding: 12px 14px;
		margin-bottom: 10px;
	}

	.cu-remote-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		flex-wrap: wrap;
	}

	.cu-remote-name {
		display: flex;
		align-items: center;
		gap: 7px;
		font-size: 14px;
		font-weight: 500;
	}

	.cu-remote-state {
		color: var(--cu-muted);
	}

	.cu-remote-state-ok {
		color: var(--cu-verified);
	}

	.cu-remote-detail {
		color: var(--cu-muted);
		font-size: 12px;
		margin-top: 2px;
	}

	.cu-remote-actions {
		display: flex;
		gap: 6px;
		flex-wrap: wrap;
	}

	.cu-vault-action {
		display: flex;
		gap: 8px;
		margin-top: 10px;
		max-width: 340px;
	}
</style>
