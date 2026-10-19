<script lang="ts">
	import Rich from './Rich.svelte';
	import { fill, i18n, plural } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import {
		appSecurityGet,
		appSecuritySet,
		driveDeniedPrograms,
		memoryLockStatus,
		type MemoryLockStatus,
		type ProgramAccess,
		panicNow,
		securityPolicyGet,
		securityPolicySet,
		type AppSecurity,
		type SecurityState,
		type VaultPolicy,
	} from '$lib/api/security';
	import { applyForcedLock } from '$lib/state/lock';
	import {
		SECURITY_OPTIONS,
		elevatedSummary,
		isAppOption,
		optionState,
		type AppToggle,
		type PolicyToggle,
		type SecurityOption,
	} from '$lib/securityOptions';
	import { refreshMount } from '$lib/state/mount.svelte';
	import { friendlyError } from '$lib/errors';

	let {
		onchanged,
	}: {
		onchanged?: (next: SecurityState) => void;
	} = $props();

	let sec = $state<SecurityState | null>(null);
	let loadError = $state<string | null>(null);
	let error = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);

	let pending = $state<VaultPolicy | null>(null);
	let reauthPassword = $state('');
	let reauthKeyPin = $state('');

	let appSec = $state<AppSecurity | null>(null);
	let appError = $state<string | null>(null);
	let shortcutDraft = $state('');
	let panicBusy = $state(false);

	const vaultOptions = SECURITY_OPTIONS.filter((o) => !isAppOption(o));
	const appOptions = SECURITY_OPTIONS.filter(isAppOption);

	let memLock = $state<MemoryLockStatus | null>(null);

	async function saveApp(next: AppSecurity) {
		appError = null;
		try {
			appSec = await appSecuritySet(next);
			shortcutDraft = appSec.panicShortcut ?? '';
			memLock = await memoryLockStatus();
		} catch (e) {
			appError = friendlyError(e);
		}
	}

	function toggleApp(field: AppToggle, input: HTMLInputElement) {
		if (!appSec) return;
		const value = input.checked;
		input.checked = appSec[field];
		void saveApp({ ...appSec, [field]: value });
	}

	function saveShortcut() {
		if (!appSec) return;
		void saveApp({ ...appSec, panicShortcut: shortcutDraft.trim() || null });
	}

	async function panic() {
		panicBusy = true;
		appError = null;
		try {
			const report = await panicNow();
			applyForcedLock(report.lock);
		} catch (e) {
			appError = friendlyError(e);
		} finally {
			panicBusy = false;
		}
	}

	let deniedPrograms = $state<string[]>([]);
	let allowedDraft = $state('');
	let exemptDraft = $state('');
	let thresholdDraft = $state('');

	async function refreshDenied() {
		try {
			deniedPrograms = await driveDeniedPrograms();
		} catch {
			deniedPrograms = [];
		}
	}

	function setAccessMode(mode: ProgramAccess, select: HTMLSelectElement) {
		if (!sec) return;
		select.value = sec.effective.programAccess;
		void apply({ ...sec.policy, programAccess: mode });
	}

	type ProgramList = 'allowedPrograms' | 'exemptPrograms';

	function addProgram(list: ProgramList, program: string) {
		const path = program.trim();
		if (!sec || !path || sec.policy[list].includes(path)) return;
		void apply({ ...sec.policy, [list]: [...sec.policy[list], path] });
		if (list === 'allowedPrograms') allowedDraft = '';
		else exemptDraft = '';
	}

	function removeProgram(list: ProgramList, program: string) {
		if (!sec) return;
		void apply({ ...sec.policy, [list]: sec.policy[list].filter((p) => p !== program) });
	}

	function saveThreshold() {
		if (!sec) return;
		const value = Number.parseInt(thresholdDraft, 10);
		if (!Number.isFinite(value) || value < 0) return;
		void apply({ ...sec.policy, massReadThreshold: value });
	}

	$effect(() => {
		if (sec) thresholdDraft = String(sec.effective.massReadThreshold);
	});

	const summary = $derived(sec ? elevatedSummary(sec.options) : { now: [], later: [] });

	onMount(async () => {
		try {
			sec = await securityPolicyGet();
			onchanged?.(sec);
			appSec = await appSecurityGet();
			shortcutDraft = appSec.panicShortcut ?? '';
			memLock = await memoryLockStatus();
			await refreshDenied();
		} catch (e) {
			loadError = friendlyError(e);
		}
	});

	async function apply(policy: VaultPolicy, password?: string, keyPin?: string) {
		error = null;
		notice = null;
		busy = true;
		try {
			const outcome = await securityPolicySet(policy, password, keyPin);
			if (outcome.status === 'passwordRequired') {
				pending = policy;
				return;
			}
			sec = outcome.state;
			pending = null;
			reauthPassword = '';
			if (outcome.revokedJobs.length > 0) {
				notice = fill(i18n.t.security.autoUnlockRevoked, { jobs: outcome.revokedJobs.join(', ') });
			}
			onchanged?.(outcome.state);
			await refreshMount();
		} catch (e) {
			error = friendlyError(e);
		} finally {
			busy = false;
		}
	}

	function toggle(field: PolicyToggle, input: HTMLInputElement) {
		if (!sec) return;
		const value = input.checked;
		input.checked = field === 'elevated' ? sec.policy.elevated : sec.effective[field];
		void apply({ ...sec.policy, [field]: value });
	}

	function confirmReauth() {
		if (!pending || !reauthPassword) return;
		const password = reauthPassword;
		const keyPin = reauthKeyPin || undefined;
		reauthPassword = '';
		reauthKeyPin = '';
		void apply(pending, password, keyPin);
	}

	function cancelReauth() {
		pending = null;
		reauthPassword = '';
		error = null;
	}

	function forcedByElevated(option: SecurityOption): boolean {
		return !!sec && sec.policy.elevated && option.inElevated;
	}
</script>

{#snippet programList(
	list: ProgramList,
	title: string,
)}
	{#if sec}
		<div class="cu-sec-extra-col">
			<span class="cu-sec-extra-title">{title}</span>
			{#each sec.policy[list] as program (program)}
				<span class="cu-sec-program">
					<span class="cu-mono">{program}</span>
					<button
						type="button"
						class="cu-btn sm"
						disabled={busy || pending !== null}
						onclick={() => removeProgram(list, program)}
					>
						{i18n.t.common.remove}
					</button>
				</span>
			{:else}
				<span class="cu-note">{i18n.t.common.none}</span>
			{/each}
		</div>
	{/if}
{/snippet}

<section class="cu-section">
	<h2>{i18n.t.security.title}</h2>
	<p class="cu-note">
		{i18n.t.security.intro}
	</p>

	{#if loadError}
		<p class="cu-error" role="alert">{loadError}</p>
	{:else if sec}
		<div class="cu-sec-master">
			<label class="cu-sec-switch">
				<input
					type="checkbox"
					checked={sec.policy.elevated}
					disabled={busy || pending !== null}
					onchange={(e) => toggle('elevated', e.currentTarget)}
				/>
				<strong>{i18n.t.security.elevated}</strong>
			</label>
			<p class="cu-note">
				{fill(i18n.t.security.elevatedNow, { list: summary.now.join(', ').toLowerCase() || i18n.t.security.elevatedNothing })}
				{#if summary.later.length > 0}
					{fill(i18n.t.security.elevatedLater, { list: summary.later.join(', ').toLowerCase() })}
				{/if}
			</p>
		</div>

		{#if pending}
			<form
				class="cu-sec-reauth"
				onsubmit={(e) => {
					e.preventDefault();
					confirmReauth();
				}}
			>
				<p>
					<Rich text={i18n.t.security.lowering} />
					{#if sec.twoFactor}
						{i18n.t.security.secondFactorNote}
					{/if}
				</p>
				<div class="cu-sec-reauth-row">
					<!-- svelte-ignore a11y_autofocus -->
					<input
						type="password"
						autocomplete="current-password"
						aria-label={i18n.t.vaultForm.password}
						placeholder={i18n.t.vaultForm.password}
						bind:value={reauthPassword}
						autofocus
					/>
					{#if sec.twoFactor}
						<input
							type="password"
							autocomplete="off"
							aria-label={i18n.t.security.keyPinAria}
							placeholder={i18n.t.security.keyPinPlaceholder}
							bind:value={reauthKeyPin}
						/>
					{/if}
					<button
						type="submit"
						class="cu-btn primary"
						disabled={busy || !reauthPassword}
					>
						{busy ? i18n.t.common.checking : i18n.t.common.confirm}
					</button>
					<button
						type="button"
						class="cu-btn"
						disabled={busy}
						onclick={cancelReauth}
					>
						{i18n.t.common.cancel}
					</button>
				</div>
			</form>
		{/if}

		{#if error}
			<p class="cu-error" role="alert">{error}</p>
		{/if}
		{#if notice}
			<p class="cu-note">{notice}</p>
		{/if}

		<ul class="cu-sec-list">
			{#each vaultOptions as option (option.id)}
				{@const availability = optionState(sec.options, option.id)}
				{@const forced = forcedByElevated(option)}
				{@const usable = availability.kind === 'available' && option.field !== null}
				<li class="cu-sec-row" data-state={availability.kind}>
					<label class="cu-sec-switch">
						{#if option.id === 'programAccess'}
							<select
								aria-label={i18n.t.security.allowedProgramsAria}
								value={sec.effective.programAccess}
								disabled={availability.kind !== 'available' || busy || pending !== null}
								onchange={(e) => setAccessMode(e.currentTarget.value as ProgramAccess, e.currentTarget)}
							>
								<option value="off">{i18n.t.security.modeAll}</option>
								<option value="allowlist">{i18n.t.security.modeList}</option>
								<option value="ask">{i18n.t.security.modeAsk}</option>
							</select>
						{:else}
							<input
								type="checkbox"
								checked={usable && option.field !== null && sec.effective[option.field]}
								disabled={!usable || forced || busy || pending !== null}
								onchange={(e) => option.field && toggle(option.field, e.currentTarget)}
							/>
						{/if}
						<strong>{option.title}</strong>
						{#if availability.kind === 'planned'}
							<span class="cu-sec-badge">{i18n.t.security.comingSoon}</span>
						{:else if availability.kind === 'unavailable'}
							<span class="cu-sec-badge">
								{i18n.t.security.unavailable}
							</span>
						{:else if forced}
							<span class="cu-sec-badge on">
								{i18n.t.security.onByElevated}
							</span>
						{/if}
					</label>
					<dl class="cu-sec-facts">
						<div><dt>{i18n.t.security.protects}</dt><dd>{option.protects}</dd></div>
						<div><dt>{i18n.t.security.doesNotProtect}</dt><dd>{option.limit}</dd></div>
						<div><dt>{i18n.t.security.costs}</dt><dd>{option.cost}</dd></div>
						{#if availability.kind === 'unavailable'}
							<div><dt>{i18n.t.security.whyNot}</dt><dd>{availability.reason}</dd></div>
						{:else if availability.kind === 'available' && availability.caveat}
							<div>
								<dt>{i18n.t.security.onThisSystem}</dt><dd>{availability.caveat}</dd>
							</div>
						{/if}
					</dl>
					{#if option.id === 'programAccess' && availability.kind === 'available'}
						{@render programList(
							'allowedPrograms',
							i18n.t.security.allowedPrograms,
						)}
						<div class="cu-sec-extra">
							<input
								type="text"
								aria-label={i18n.t.security.allowProgramAria}
								placeholder={i18n.t.security.programPlaceholder}
								bind:value={allowedDraft}
							/>
							<button
								type="button"
								class="cu-btn"
								disabled={busy || pending !== null || !allowedDraft.trim()}
								onclick={() => addProgram('allowedPrograms', allowedDraft)}
							>
								{i18n.t.security.allow}
							</button>
						</div>
						<div class="cu-sec-extra-col">
							<span class="cu-sec-extra-title">
								{i18n.t.security.deniedSinceMount}
								<button type="button" class="cu-btn sm" onclick={refreshDenied}>
									{i18n.t.common.refresh}
								</button>
							</span>
							{#each deniedPrograms.filter((p) => !sec?.policy.allowedPrograms.includes(p)) as program (program)}
								<span class="cu-sec-program">
									<span class="cu-mono">{program}</span>
									<button
										type="button"
										class="cu-btn sm"
										disabled={busy || pending !== null}
										onclick={() => addProgram('allowedPrograms', program)}
									>
										{i18n.t.security.allow}
									</button>
								</span>
							{:else}
								<span class="cu-note">{i18n.t.common.none}</span>
							{/each}
						</div>
						<p class="cu-note">
							{i18n.t.security.programsNote}
						</p>
					{/if}
					{#if option.id === 'decoys' && availability.kind === 'available'}
						<div class="cu-sec-extra">
							<label class="cu-sec-inline">
								{i18n.t.security.alarmBefore}
								<input
									type="number"
									min="0"
									bind:value={thresholdDraft}
								/>
								{i18n.t.security.alarmAfter}
							</label>
							<button
								type="button"
								class="cu-btn"
								disabled={busy || pending !== null}
								onclick={saveThreshold}
							>
								{i18n.t.common.save}
							</button>
						</div>
						{@render programList(
							'exemptPrograms',
							i18n.t.security.alarmExempt,
						)}
						<div class="cu-sec-extra">
							<input
								type="text"
								aria-label={i18n.t.security.exemptProgramAria}
								placeholder={i18n.t.security.programPlaceholder}
								bind:value={exemptDraft}
							/>
							<button
								type="button"
								class="cu-btn"
								disabled={busy || pending !== null || !exemptDraft.trim()}
								onclick={() => addProgram('exemptPrograms', exemptDraft)}
							>
								{i18n.t.security.exempt}
							</button>
						</div>
					{/if}
				</li>
			{/each}
		</ul>

		<h3 class="cu-sec-sub">{i18n.t.security.thisComputer}</h3>
		<p class="cu-note">
			{i18n.t.security.thisComputerNote}
		</p>
		{#if appError}
			<p class="cu-error" role="alert">{appError}</p>
		{/if}
		<ul class="cu-sec-list">
			{#each appOptions as option (option.id)}
				{@const availability = optionState(sec.options, option.id)}
				{@const usable = availability.kind === 'available' && appSec !== null}
				<li class="cu-sec-row" data-state={availability.kind}>
					<label class="cu-sec-switch">
						{#if option.appField}
							{@const field = option.appField}
							<input
								type="checkbox"
								checked={usable && appSec !== null && appSec[field]}
								disabled={!usable}
								onchange={(e) => toggleApp(field, e.currentTarget)}
							/>
						{/if}
						<strong>{option.title}</strong>
						{#if availability.kind === 'planned'}
							<span class="cu-sec-badge">{i18n.t.security.comingSoon}</span>
						{:else if availability.kind === 'unavailable'}
							<span class="cu-sec-badge">
								{i18n.t.security.unavailable}
							</span>
						{/if}
					</label>
					<dl class="cu-sec-facts">
						<div><dt>{i18n.t.security.protects}</dt><dd>{option.protects}</dd></div>
						<div><dt>{i18n.t.security.doesNotProtect}</dt><dd>{option.limit}</dd></div>
						<div><dt>{i18n.t.security.costs}</dt><dd>{option.cost}</dd></div>
						{#if availability.kind === 'unavailable'}
							<div><dt>{i18n.t.security.whyNot}</dt><dd>{availability.reason}</dd></div>
						{/if}
					</dl>
					{#if option.id === 'lockMemory' && appSec?.lockMemory}
						<p class="cu-note cu-sec-status">
							{#if memLock === null}
								{i18n.t.security.fromNextUnlock}
							{:else if memLock.failed === 0}
								{plural(memLock.locked, i18n.t.security.memLockActive)}
							{:else}
								{fill(i18n.t.security.memLockFailed, { failed: memLock.failed, total: memLock.locked + memLock.failed })}
							{/if}
						</p>
					{/if}
					{#if option.id === 'panic' && usable}
						<div class="cu-sec-panic">
							<button
								type="button"
								class="cu-btn danger"
								disabled={panicBusy}
								onclick={panic}
							>
								{panicBusy ? i18n.t.common.working : i18n.t.security.panicNow}
							</button>
							<input
								type="text"
								aria-label={i18n.t.security.panicShortcutAria}
								placeholder={i18n.t.security.panicShortcutPlaceholder}
								bind:value={shortcutDraft}
							/>
							<button type="button" class="cu-btn" onclick={saveShortcut}>
								{i18n.t.security.saveShortcut}
							</button>
						</div>
						<p class="cu-note">
							<Rich text={i18n.t.security.shortcutNote} />
						</p>
					{/if}
				</li>
			{/each}
		</ul>

		<p class="cu-note">
			{i18n.t.security.limit}
		</p>
	{/if}
</section>

<style>
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

	.cu-note {
		color: var(--cu-muted);
		font-size: 13px;
		margin: 8px 0 0;
	}

	.cu-sec-master {
		margin: 16px 0 4px;
		padding: 12px 14px;
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		background: var(--cu-surface);
	}

	.cu-sec-switch {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
		font-size: 13px;
	}

	.cu-sec-switch input {
		width: auto;
		margin: 0;
	}

	.cu-sec-badge {
		font-size: 11px;
		padding: 1px 8px;
		border: 1px solid var(--cu-line-strong);
		border-radius: 999px;
		color: var(--cu-muted);
		white-space: nowrap;
	}

	.cu-sec-badge.on {
		color: var(--cu-verified);
		border-color: currentColor;
	}

	.cu-sec-list {
		list-style: none;
		margin: 12px 0 0;
		padding: 0;
	}

	.cu-sec-row {
		padding: 12px 0;
		border-top: 1px solid var(--cu-line);
	}

	.cu-sec-row[data-state='planned'] .cu-sec-facts,
	.cu-sec-row[data-state='unavailable'] .cu-sec-facts {
		opacity: 0.75;
	}

	.cu-sec-facts {
		margin: 8px 0 0 24px;
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-sec-facts div {
		display: flex;
		gap: 8px;
		margin-top: 2px;
	}

	.cu-sec-facts dt {
		flex: 0 0 96px;
		color: var(--cu-text);
	}

	.cu-sec-facts dd {
		margin: 0;
		min-width: 0;
	}

	.cu-sec-status {
		margin-inline-start: 24px;
	}

	.cu-sec-sub {
		margin: 24px 0 0;
		font-size: 13px;
	}

	.cu-sec-panic {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		margin: 10px 0 0 24px;
	}

	.cu-sec-panic input {
		flex: 1;
		min-width: 180px;
		height: 34px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		color: var(--cu-text);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		font-size: 13px;
	}

	.cu-sec-switch select,
	.cu-sec-extra input,
	.cu-sec-inline input {
		height: 34px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		color: var(--cu-text);
		border-radius: var(--cu-radius-sm);
		padding: 0 10px;
		font-size: 13px;
	}

	.cu-sec-extra {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		margin: 10px 0 0 24px;
	}

	.cu-sec-extra > input {
		flex: 1;
		min-width: 200px;
	}

	.cu-sec-inline {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
		font-size: 13px;
	}

	.cu-sec-inline input {
		width: 90px;
	}

	.cu-sec-extra-col {
		display: flex;
		flex-direction: column;
		gap: 6px;
		margin: 10px 0 0 24px;
		font-size: 13px;
	}

	.cu-sec-extra-title {
		display: flex;
		align-items: center;
		gap: 8px;
		color: var(--cu-text);
		font-size: 12px;
	}

	.cu-sec-program {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}

	.cu-sec-program .cu-mono {
		min-width: 0;
		overflow-wrap: anywhere;
	}

	.cu-sec-reauth {
		margin: 12px 0 0;
		padding: 12px 14px;
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		font-size: 13px;
	}

	.cu-sec-reauth p {
		margin: 0 0 10px;
	}

	.cu-sec-reauth-row {
		display: flex;
		gap: 8px;
		align-items: center;
	}

	.cu-sec-reauth-row input {
		flex: 1;
		min-width: 0;
		height: 34px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		color: var(--cu-text);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		font-size: 13px;
	}
</style>
