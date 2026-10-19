<script lang="ts">
	import { fill, i18n, plural } from '$lib/i18n/index.svelte';
	import { onMount, tick } from 'svelte';
	import { friendlyError, describeMessage } from '$lib/errors';
	import { open } from '@tauri-apps/plugin-dialog';
	import {
		jobList,
		jobAdd,
		jobRemove,
		jobSetEnabled,
		jobEnableAutoUnlock,
		jobHistory,
		jobTopLevelFolders,
		jobSetExcludes,
		jobSetScheduleWindows,
		jobSetPreHook,
		jobSetPostHook,
		jobSetAnomalyGuard,
		jobSetTiming,
		type Job,
		type RunRow,
		type ConflictInfo,
		type ScheduleWindow,
		type HookSpec,
		type AnomalyGuard,
	} from '$lib/api/jobs';
	import { jobVerify, type JobVerifyResult } from '$lib/api/versions';
	import { jobsServiceStatus, type ServiceJobStatus } from '$lib/api/overview';
	import { session } from '$lib/state/session.svelte';
	import { startRun, cancelRun, sync } from '$lib/state/sync.svelte';
	import { formatAgo, formatBytes, formatIn, formatSpeed } from '$lib/format';

	const DEFAULT_KEEP_VERSIONS = 5;
	const DEFAULT_KEEP_RUNS = 500;

	let jobs = $state<Job[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let showCreate = $state(false);
	let newName = $state('');
	let newLocalRoot = $state('');
	let newVaultPath = $state('');
	let newIntervalMinutes = $state(30);

	const activeRun = $derived(sync.run);
	const lastSummary = $derived(
		sync.last && (sync.last.kind === 'completed' || sync.last.kind === 'cancelled') ? sync.last : null,
	);

	let lastRuns = $state<Record<string, RunRow | undefined>>({});
	let serviceStatuses = $state<ServiceJobStatus[]>([]);

	async function loadJobStatus() {
		const root = session.vaultRoot;
		if (!root || jobs.length === 0) return;
		try {
			const [rows, statuses] = await Promise.all([
				Promise.all(jobs.map((j) => jobHistory(j.name, 1).catch(() => [] as RunRow[]))),
				jobsServiceStatus(root).catch(() => [] as ServiceJobStatus[]),
			]);
			lastRuns = Object.fromEntries(jobs.map((j, i) => [j.name, rows[i][0]]));
			serviceStatuses = statuses;
		} catch {
		}
	}

	$effect(() => {
		void jobs;
		void sync.version;
		loadJobStatus();
	});

	function nextRunText(name: string): string {
		const s = serviceStatuses.find((x) => x.job === name);
		if (s && s.serviceActive && s.nextRun !== null) return fill(i18n.t.jobs.nextRun, { when: formatIn(s.nextRun) });
		return i18n.t.jobs.autoOff;
	}

	function lastRunDetail(row: RunRow): string {
		const parts: string[] = [];
		if (row.filesUploaded) parts.push(plural(row.filesUploaded, i18n.t.activity.uploaded));
		if (row.filesRenamed) parts.push(plural(row.filesRenamed, i18n.t.activity.renamed));
		const when = formatAgo((row.finishedAt ?? row.startedAt) / 1000);
		return parts.length ? `${when}, ${parts.join(', ')}` : when;
	}

	let autoUnlockFor = $state<string | null>(null);
	let autoUnlockPassword = $state('');

	let historyFor = $state<string | null>(null);
	let historyRows = $state<RunRow[]>([]);

	let verifyFor = $state<string | null>(null);
	let verifyRunning = $state(false);
	let verifyResult = $state<JobVerifyResult | null>(null);
	let verifyError = $state<string | null>(null);

	function toggleVerify(name: string) {
		verifyFor = verifyFor === name ? null : name;
		verifyResult = null;
		verifyError = null;
	}

	async function runVerify(name: string, level: 1 | 2) {
		verifyRunning = true;
		verifyResult = null;
		verifyError = null;
		session.jobRunning = true;
		try {
			verifyResult = await jobVerify(name, level);
		} catch (e) {
			verifyError = friendlyError(e);
		} finally {
			verifyRunning = false;
			session.jobRunning = false;
		}
	}

	let excludesFor = $state<string | null>(null);
	let excludesFolders = $state<string[]>([]);
	let newExcludePattern = $state('');

	const DAY_LABELS = $derived(i18n.t.jobs.days);
	let scheduleFor = $state<string | null>(null);
	let newWindowDays = $state<boolean[]>(new Array(7).fill(false));
	let newWindowFrom = $state('22:00');
	let newWindowTo = $state('06:00');

	let hooksFor = $state<string | null>(null);
	let preProgram = $state('');
	let preArgs = $state('');
	let preTimeoutSecs = $state(60);
	let postProgram = $state('');
	let postArgs = $state('');
	let postTimeoutSecs = $state(60);

	let anomalyBlockedFor = $state<{
		name: string;
		reason: string;
		uploadsAffected: number;
		deletesAffected: number;
		renamesAffected: number;
	} | null>(null);
	let anomalyGuardFor = $state<string | null>(null);
	let anomalyGuardDraft = $state<AnomalyGuard | null>(null);
	let timingFor = $state<string | null>(null);
	let timingShuffle = $state(false);
	let timingDelayMinutes = $state(0);

	async function load() {
		loading = true;
		error = null;
		try {
			jobs = await jobList();
		} catch (e) {
			error = friendlyError(e);
		} finally {
			loading = false;
		}
	}

	load();

	async function pickLocalRoot() {
		const picked = await open({ directory: true, multiple: false });
		if (typeof picked === 'string') newLocalRoot = picked;
	}

	async function createJob() {
		error = null;
		try {
			await jobAdd(
				newName.trim(),
				newLocalRoot,
				newVaultPath.trim(),
				Math.max(60, Math.round(newIntervalMinutes * 60)),
				DEFAULT_KEEP_VERSIONS,
				DEFAULT_KEEP_RUNS,
			);
			showCreate = false;
			newName = '';
			newLocalRoot = '';
			newVaultPath = '';
			newIntervalMinutes = 30;
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	let confirmingRemove = $state<string | null>(null);

	let openMenuFor = $state<string | null>(null);

	async function toggleMenu(name: string) {
		openMenuFor = openMenuFor === name ? null : name;
		if (!openMenuFor) return;
		await tick();
		menuItems()[0]?.focus();
	}

	function menuItems(): HTMLElement[] {
		return Array.from(document.querySelectorAll<HTMLElement>('.cu-job-menu [role="menuitem"]'));
	}

	function closeMenu(returnFocus = false) {
		const name = openMenuFor;
		openMenuFor = null;
		if (returnFocus && name) {
			document.querySelector<HTMLElement>(`[data-job-menu="${CSS.escape(name)}"]`)?.focus();
		}
	}

	function onMenuKeydown(e: KeyboardEvent) {
		const items = menuItems();
		if (!items.length) return;
		const current = items.indexOf(document.activeElement as HTMLElement);
		let next: number | null = null;
		if (e.key === 'ArrowDown') next = (current + 1) % items.length;
		else if (e.key === 'ArrowUp') next = (current - 1 + items.length) % items.length;
		else if (e.key === 'Home') next = 0;
		else if (e.key === 'End') next = items.length - 1;
		else if (e.key === 'Tab') closeMenu();
		if (next !== null) {
			e.preventDefault();
			items[next].focus();
		}
	}

	onMount(() => {
		const onWindowClick = (e: MouseEvent) => {
			if (!openMenuFor) return;
			if (!(e.target as HTMLElement).closest('.cu-job-menu-wrap')) closeMenu();
		};
		const onWindowKeydown = (e: KeyboardEvent) => {
			if (e.key === 'Escape' && openMenuFor) closeMenu(true);
		};
		window.addEventListener('click', onWindowClick);
		window.addEventListener('keydown', onWindowKeydown);
		return () => {
			window.removeEventListener('click', onWindowClick);
			window.removeEventListener('keydown', onWindowKeydown);
		};
	});

	async function remove(name: string) {
		confirmingRemove = null;
		try {
			await jobRemove(name);
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function toggleEnabled(job: Job) {
		try {
			await jobSetEnabled(job.name, !job.enabled);
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function runNow(name: string, confirmAnomaly = false) {
		error = null;
		if (!confirmAnomaly) anomalyBlockedFor = null;
		const outcome = await startRun(name, confirmAnomaly);
		if (outcome?.kind === 'anomalyBlocked') {
			anomalyBlockedFor = {
				name,
				reason: outcome.reason,
				uploadsAffected: outcome.uploadsAffected,
				deletesAffected: outcome.deletesAffected,
				renamesAffected: outcome.renamesAffected,
			};
		} else if (!outcome && sync.last?.kind === 'failed' && sync.last.name === name) {
			error = sync.last.text;
		}
	}

	async function confirmAnomalyOverride() {
		if (!anomalyBlockedFor) return;
		const name = anomalyBlockedFor.name;
		anomalyBlockedFor = null;
		await runNow(name, true);
	}

	async function confirmAutoUnlock() {
		if (!autoUnlockFor) return;
		try {
			await jobEnableAutoUnlock(autoUnlockFor, autoUnlockPassword);
			autoUnlockFor = null;
			autoUnlockPassword = '';
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function toggleHistory(name: string) {
		if (historyFor === name) {
			historyFor = null;
			return;
		}
		historyFor = name;
		try {
			historyRows = await jobHistory(name, 10);
		} catch (e) {
			error = friendlyError(e);
		}
	}

	function formatDate(ms: number): string {
		return new Date(ms).toLocaleString(i18n.language.tag);
	}

	const outcomeKeys = { ok: 'outcomeOk', failed: 'outcomeFailed', interrupted: 'outcomeInterrupted' } as const;

	function folderPattern(folderName: string): string {
		return `/${folderName.replace(/[\\*?]/g, '\\$&')}/`;
	}

	function legacyFolderPattern(folderName: string): string {
		return `${folderName}/`;
	}

	function isFolderExcluded(job: Job, folderName: string): boolean {
		return (
			job.excludes.includes(folderPattern(folderName)) ||
			job.excludes.includes(legacyFolderPattern(folderName))
		);
	}

	async function toggleExcludes(name: string) {
		if (excludesFor === name) {
			excludesFor = null;
			return;
		}
		excludesFor = name;
		newExcludePattern = '';
		excludesFolders = [];
		try {
			excludesFolders = await jobTopLevelFolders(name);
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function toggleFolderExcluded(job: Job, folderName: string) {
		const pattern = folderPattern(folderName);
		const legacy = legacyFolderPattern(folderName);
		const excludes = isFolderExcluded(job, folderName)
			? job.excludes.filter((p) => p !== pattern && p !== legacy)
			: [...job.excludes, pattern];
		try {
			await jobSetExcludes(job.name, excludes);
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function addFreeExcludePattern(job: Job) {
		const pattern = newExcludePattern.trim();
		if (!pattern || job.excludes.includes(pattern)) {
			newExcludePattern = '';
			return;
		}
		try {
			await jobSetExcludes(job.name, [...job.excludes, pattern]);
			newExcludePattern = '';
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function removeExcludePattern(job: Job, pattern: string) {
		try {
			await jobSetExcludes(
				job.name,
				job.excludes.filter((p) => p !== pattern),
			);
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	function minutesToTimeInput(min: number): string {
		const h = Math.floor(min / 60) % 24;
		const m = min % 60;
		return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
	}

	function timeInputToMinutes(value: string): number | null {
		const match = /^(\d{1,2}):(\d{2})$/.exec(value);
		if (!match) return null;
		const h = Number(match[1]);
		const m = Number(match[2]);
		if (h > 23 || m > 59) return null;
		return h * 60 + m;
	}

	function formatScheduleWindow(w: ScheduleWindow): string {
		const days = DAY_LABELS.filter((_, i) => (w.days & (1 << i)) !== 0).join(',');
		return `${days} ${minutesToTimeInput(w.fromMin)}–${minutesToTimeInput(w.toMin)}`;
	}

	function toggleSchedule(name: string) {
		if (scheduleFor === name) {
			scheduleFor = null;
			return;
		}
		scheduleFor = name;
		scheduleError = null;
		newWindowDays = new Array(7).fill(false);
		newWindowFrom = '22:00';
		newWindowTo = '06:00';
	}

	let scheduleError = $state<string | null>(null);

	async function addScheduleWindow(job: Job) {
		scheduleError = null;
		let days = 0;
		newWindowDays.forEach((checked, i) => {
			if (checked) days |= 1 << i;
		});
		if (days === 0) {
			scheduleError = i18n.t.jobs.windowNeedsDay;
			return;
		}
		const fromMin = timeInputToMinutes(newWindowFrom);
		const toMin = timeInputToMinutes(newWindowTo);
		if (fromMin === null || toMin === null) {
			scheduleError = i18n.t.jobs.windowNeedsTimes;
			return;
		}
		const window: ScheduleWindow = { days, fromMin, toMin };
		try {
			await jobSetScheduleWindows(job.name, [...job.scheduleWindows, window]);
			newWindowDays = new Array(7).fill(false);
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function removeScheduleWindow(job: Job, index: number) {
		try {
			await jobSetScheduleWindows(
				job.name,
				job.scheduleWindows.filter((_, i) => i !== index),
			);
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	function splitArgs(value: string): string[] {
		return value.split(/\s+/).filter((a) => a.length > 0);
	}

	function joinArgs(args: string[]): string {
		return args.join(' ');
	}

	function toggleHooks(job: Job) {
		if (hooksFor === job.name) {
			hooksFor = null;
			return;
		}
		hooksFor = job.name;
		preProgram = job.preHook?.program ?? '';
		preArgs = joinArgs(job.preHook?.args ?? []);
		preTimeoutSecs = job.preHook?.timeoutSecs ?? 60;
		postProgram = job.postHook?.program ?? '';
		postArgs = joinArgs(job.postHook?.args ?? []);
		postTimeoutSecs = job.postHook?.timeoutSecs ?? 60;
	}

	async function savePreHook(job: Job) {
		const program = preProgram.trim();
		if (!program) {
			error = i18n.t.jobs.preHookNeedsProgram;
			return;
		}
		const hook: HookSpec = { program, args: splitArgs(preArgs), timeoutSecs: preTimeoutSecs };
		try {
			await jobSetPreHook(job.name, hook);
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function clearPreHook(job: Job) {
		try {
			await jobSetPreHook(job.name, null);
			preProgram = '';
			preArgs = '';
			preTimeoutSecs = 60;
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function savePostHook(job: Job) {
		const program = postProgram.trim();
		if (!program) {
			error = i18n.t.jobs.postHookNeedsProgram;
			return;
		}
		const hook: HookSpec = { program, args: splitArgs(postArgs), timeoutSecs: postTimeoutSecs };
		try {
			await jobSetPostHook(job.name, hook);
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function clearPostHook(job: Job) {
		try {
			await jobSetPostHook(job.name, null);
			postProgram = '';
			postArgs = '';
			postTimeoutSecs = 60;
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	function toggleAnomalyGuard(job: Job) {
		if (anomalyGuardFor === job.name) {
			anomalyGuardFor = null;
			anomalyGuardDraft = null;
			return;
		}
		anomalyGuardFor = job.name;
		anomalyGuardDraft = { ...job.anomalyGuard };
	}

	function toggleTiming(job: Job) {
		if (timingFor === job.name) {
			timingFor = null;
			return;
		}
		timingFor = job.name;
		timingShuffle = job.timing.shuffleUploads;
		timingDelayMinutes = Math.round(job.timing.randomDelayMaxSecs / 60);
	}

	async function saveTiming(job: Job) {
		try {
			const minutes = Math.min(1440, Math.max(0, Math.round(timingDelayMinutes || 0)));
			await jobSetTiming(job.name, { shuffleUploads: timingShuffle, randomDelayMaxSecs: minutes * 60 });
			timingFor = null;
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function saveAnomalyGuard(job: Job) {
		if (!anomalyGuardDraft) return;
		try {
			await jobSetAnomalyGuard(job.name, anomalyGuardDraft);
			anomalyGuardFor = null;
			anomalyGuardDraft = null;
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}
</script>

<div class="cu-page">
	<div class="cu-toolbar">
		<h1 class="cu-page-title">{i18n.t.nav.jobs}</h1>
		<button type="button" class="cu-btn" onclick={() => (showCreate = !showCreate)}>{i18n.t.jobs.newJob}</button>
	</div>

	{#if showCreate}
		<form
			class="cu-new-job"
			onsubmit={(e) => {
				e.preventDefault();
				createJob();
			}}
		>
			<label class="cu-field">
				<span>{i18n.t.jobs.name}</span>
				<input type="text" bind:value={newName} placeholder={i18n.t.jobs.namePlaceholder} required />
			</label>
			<label class="cu-field">
				<span>{i18n.t.jobs.localFolder}</span>
				<div class="cu-path-row">
					<input class="cu-mono" type="text" bind:value={newLocalRoot} placeholder={i18n.t.jobs.localPlaceholder} required />
					<button type="button" class="cu-btn" onclick={pickLocalRoot}>{i18n.t.common.choose}</button>
				</div>
			</label>
			<label class="cu-field">
				<span>{i18n.t.jobs.vaultPath}</span>
				<input class="cu-mono" type="text" bind:value={newVaultPath} placeholder={i18n.t.jobs.vaultPathPlaceholder} />
			</label>
			<label class="cu-field">
				<span>{i18n.t.jobs.intervalMinutes}</span>
				<input type="number" min="1" bind:value={newIntervalMinutes} />
			</label>
			<button type="submit" class="cu-btn primary">{i18n.t.jobs.create}</button>
		</form>
	{/if}

	{#if error}
		<p class="cu-error" role="alert">{error}</p>
	{/if}

	{#if loading}
		<p class="cu-note">{i18n.t.common.loading}</p>
	{:else if jobs.length === 0}
		<p class="cu-note">{i18n.t.jobs.empty}</p>
	{:else}
		<ul class="cu-jobs">
			{#each jobs as job (job.name)}
				<li>
					<div class="cu-job-row">
						<div class="cu-job-col cu-job-col-name">
							<div class="cu-job-name">
								{job.name}
								{#if !job.enabled}<span class="cu-badge">{i18n.t.jobs.badgeDisabled}</span>{/if}
								{#if !job.matchesOpenVault}<span class="cu-badge">{i18n.t.jobs.badgeOtherVault}</span>{/if}
								{#if job.autoUnlock}<span class="cu-badge cu-badge-ok">{i18n.t.jobs.badgeAutoUnlock}</span>{/if}
							</div>
							<div class="cu-job-detail cu-mono">
								{fill(i18n.t.jobs.summaryLine, { local: job.localRoot, vault: job.vaultPath || '/', minutes: Math.round(job.intervalSecs / 60) })}
							</div>
						</div>

						<div class="cu-job-col cu-job-col-status">
							{#if sync.run?.name === job.name}
								{@const pct = sync.run.total > 0 ? Math.round((sync.run.index / sync.run.total) * 100) : 0}
								<div class="cu-job-status-line">
									<svg class="cu-i"><use href="#cu-i-sync" /></svg>
									<span>{fill(i18n.t.jobs.runningPercent, { percent: pct })}</span>
								</div>
								<div class="cu-job-bar"><b style:width={`${pct}%`}></b></div>
								<div class="cu-job-status-sub">
									{plural(Math.max(0, sync.run.total - sync.run.index), i18n.t.jobs.filesLeft)}{sync.run.speed > 0 ? `, ${formatSpeed(sync.run.speed)}` : ""}
								</div>
							{:else}
								{@const row = lastRuns[job.name]}
								{#if !row}
									<div class="cu-job-status-line cu-job-status-muted">
										<svg class="cu-i"><use href="#cu-i-clock" /></svg>
										<span>{i18n.t.home.stateNever}</span>
									</div>
								{:else if row.outcome === 'ok'}
									<div class="cu-job-status-line cu-job-status-ok">
										<svg class="cu-i"><use href="#cu-i-check" /></svg>
										<span>{i18n.t.jobs.lastOk}</span>
									</div>
									<div class="cu-job-status-sub">{lastRunDetail(row)}</div>
									<div class="cu-job-status-sub">{nextRunText(job.name)}</div>
								{:else if row.outcome === 'failed'}
									<div class="cu-job-status-line cu-job-status-warn">
										<svg class="cu-i"><use href="#cu-i-warn" /></svg>
										<span>{i18n.t.syncStrip.failed}</span>
									</div>
									<div class="cu-job-status-sub">{formatAgo((row.finishedAt ?? row.startedAt) / 1000)}</div>
								{:else if row.outcome === 'interrupted'}
									<div class="cu-job-status-line cu-job-status-muted">
										<svg class="cu-i"><use href="#cu-i-pause" /></svg>
										<span>{i18n.t.home.statePaused}</span>
									</div>
									<div class="cu-job-status-sub">{i18n.t.jobs.resumeToContinue}</div>
								{:else}
									<div class="cu-job-status-line cu-job-status-muted">
										<svg class="cu-i"><use href="#cu-i-clock" /></svg>
										<span>{i18n.t.home.stateNever}</span>
									</div>
								{/if}
							{/if}
						</div>

						<div class="cu-job-col cu-job-col-actions">
							{#if job.matchesOpenVault}
								{#if sync.run?.name === job.name}
									<button
										type="button"
										class="cu-btn sm"
										onclick={cancelRun}
										disabled={sync.run.cancelling}
									>
										{sync.run.cancelling ? i18n.t.jobs.pausingShort : i18n.t.syncStrip.pause}
									</button>
								{:else}
									<button
										type="button"
										class="cu-btn primary sm"
										onclick={() => runNow(job.name)}
										disabled={session.jobRunning}
									>
										{lastRuns[job.name]?.outcome === 'interrupted' ? i18n.t.syncStrip.resume : i18n.t.jobs.runNow}
									</button>
								{/if}
							{/if}
							<div class="cu-job-menu-wrap">
								<button
									type="button"
									class="cu-icon-btn"
									data-job-menu={job.name}
									aria-haspopup="true"
									aria-expanded={openMenuFor === job.name}
									aria-label={i18n.t.jobs.moreActions}
									onclick={() => toggleMenu(job.name)}
								>
									<svg class="cu-i"><use href="#cu-i-more" /></svg>
								</button>
								{#if openMenuFor === job.name}
									<div class="cu-job-menu" role="menu" tabindex="-1" onkeydown={onMenuKeydown}>
										<button role="menuitem" onclick={() => { toggleEnabled(job); closeMenu(); }}>
											{job.enabled ? i18n.t.jobs.disable : i18n.t.jobs.enable}
										</button>
										<button role="menuitem" onclick={() => { toggleHistory(job.name); closeMenu(); }}>
											{i18n.t.jobs.history}
										</button>
										<button
											role="menuitem"
											onclick={() => { toggleVerify(job.name); closeMenu(); }}
										>
											{i18n.t.jobs.verify}
										</button>
										<button
											role="menuitem"
											onclick={() => { toggleExcludes(job.name); closeMenu(); }}
										>
											{i18n.t.jobs.excludes}
										</button>
										<button
											role="menuitem"
											onclick={() => { toggleSchedule(job.name); closeMenu(); }}
										>
											{i18n.t.jobs.windows}
										</button>
										<button
											role="menuitem"
											onclick={() => { toggleHooks(job); closeMenu(); }}
										>
											{i18n.t.jobs.hooks}
										</button>
										<button
											role="menuitem"
											onclick={() => { toggleAnomalyGuard(job); closeMenu(); }}
										>
											{i18n.t.jobs.antiRansomware}
										</button>
										<button
											role="menuitem"
											onclick={() => { toggleTiming(job); closeMenu(); }}
										>
											{i18n.t.jobs.blurTiming}
										</button>
										{#if !job.autoUnlock}
											<button
												role="menuitem"
												onclick={() => { autoUnlockFor = job.name; closeMenu(); }}
											>
												{i18n.t.jobs.enableAutoUnlock}
											</button>
										{/if}
										<div class="cu-job-menu-sep"></div>
										<button
											role="menuitem"
											class="cu-job-menu-danger"
											onclick={() => { confirmingRemove = job.name; closeMenu(); }}
										>
											{i18n.t.common.removeEntry}
										</button>
									</div>
								{/if}
							</div>
						</div>
					</div>

					{#if confirmingRemove === job.name}
						<div class="cu-warning">
							<strong>{fill(i18n.t.jobs.removeQuestion, { name: job.name })}</strong>
							<p>{i18n.t.jobs.removeNote}</p>
							<div class="cu-warning-actions">
								<button
									type="button"
									class="cu-btn sm danger"
									onclick={() => remove(job.name)}
								>
									{i18n.t.jobs.confirmRemove}
								</button>
								<button
									type="button"
									class="cu-btn sm"
									onclick={() => (confirmingRemove = null)}
								>
									{i18n.t.common.cancel}
								</button>
							</div>
						</div>
					{/if}

					{#if activeRun?.name === job.name}
						<div class="cu-transfer">
							<span>
								{fill(i18n.t.jobs.itemOf, { index: activeRun.index, total: activeRun.total })} <span class="cu-mono">{activeRun.file}</span>
							</span>
							<span class="cu-mono">{formatBytes(activeRun.doneBytes + activeRun.fileBytes)}</span>
						</div>
					{/if}

					{#if lastSummary?.name === job.name}
						<p class="cu-summary">{lastSummary.text}</p>
						{#if lastSummary.postHookText}
							<p class="cu-note">
								{lastSummary.postHookText}
							</p>
						{/if}
						{#if lastSummary.conflicts.length > 0}
							<div class="cu-warning">
								<strong
									>{plural(lastSummary.conflicts.length, i18n.t.jobs.conflictsAuto)}</strong
								>
								<ul class="cu-conflict-list">
									{#each lastSummary.conflicts as c (c.vaultPath)}
										<li><span class="cu-mono">{c.vaultPath}</span>: {describeMessage(c.detail)}</li>
									{/each}
								</ul>
							</div>
						{/if}
					{/if}

					{#if anomalyBlockedFor && anomalyBlockedFor.name === job.name}
						<div class="cu-warning">
							<strong>{i18n.t.jobs.anomalyTitle}</strong>
							<p>{describeMessage(anomalyBlockedFor.reason)}</p>
							<p>
								{fill(i18n.t.jobs.anomalyBody, { uploads: anomalyBlockedFor.uploadsAffected, deletes: anomalyBlockedFor.deletesAffected, renames: anomalyBlockedFor.renamesAffected })}
							</p>
							<div class="cu-hook-actions">
								<button
									type="button"
									class="cu-btn sm danger"
									onclick={confirmAnomalyOverride}
								>
									{i18n.t.jobs.confirmAnyway}
								</button>
								<button
									type="button"
									class="cu-btn sm"
									onclick={() => (anomalyBlockedFor = null)}
								>
									{i18n.t.common.cancel}
								</button>
							</div>
						</div>
					{/if}

					{#if anomalyGuardFor === job.name && anomalyGuardDraft}
						<div class="cu-hooks">
							<p class="cu-note">
								{i18n.t.jobs.anomalyIntro}
							</p>
							<label class="cu-checkbox">
								<input
									type="checkbox"
									bind:checked={anomalyGuardDraft.enabled}
								/>
								<span>{i18n.t.jobs.anomalyEnabled}</span>
							</label>
							{#if anomalyGuardDraft.enabled}
								<div class="cu-hook-block">
									<label class="cu-field">
										<span>{i18n.t.jobs.anomalyUploadThreshold}</span>
										<input type="number" min="1" bind:value={anomalyGuardDraft.massModifyAbs} />
									</label>
									<label class="cu-field">
										<span>{i18n.t.jobs.anomalyUploadMultiplier}</span>
										<input
											type="number"
											min="1"
											bind:value={anomalyGuardDraft.massModifyMultiplier}
										/>
									</label>
									<label class="cu-field">
										<span>{i18n.t.jobs.anomalyDeleteThreshold}</span>
										<input type="number" min="1" bind:value={anomalyGuardDraft.massDeleteAbs} />
									</label>
									<label class="cu-field">
										<span>{i18n.t.jobs.anomalyDeleteMultiplier}</span>
										<input
											type="number"
											min="1"
											bind:value={anomalyGuardDraft.massDeleteMultiplier}
										/>
									</label>
									<label class="cu-field">
										<span>{i18n.t.jobs.anomalyRenameMin}</span>
										<input type="number" min="1" bind:value={anomalyGuardDraft.renameMinCount} />
									</label>
									<label class="cu-field">
										<span>{i18n.t.jobs.anomalyRenamePercent}</span>
										<input
											type="number"
											min="1"
											max="100"
											bind:value={anomalyGuardDraft.renameUniformFractionPct}
										/>
									</label>
									<label class="cu-field">
										<span>{i18n.t.jobs.anomalyMinRuns}</span>
										<input type="number" min="0" bind:value={anomalyGuardDraft.minBaselineRuns} />
									</label>
								</div>
							{/if}
							<div class="cu-hook-actions">
								<button
									type="button"
									class="cu-btn primary"
									onclick={() => saveAnomalyGuard(job)}
								>
									{i18n.t.common.save}
								</button>
								<button type="button" class="cu-btn" onclick={() => toggleAnomalyGuard(job)}>{i18n.t.common.cancel}</button>
							</div>
						</div>
					{/if}

					{#if timingFor === job.name}
						<div class="cu-hooks">
							<p class="cu-note">
								{i18n.t.jobs.blurIntro}
							</p>
							<label class="cu-checkbox">
								<input
									type="checkbox"
									bind:checked={timingShuffle}
								/>
								<span>{i18n.t.jobs.blurShuffle}</span>
							</label>
							<label class="cu-field">
								<span>{i18n.t.jobs.blurDelay}</span>
								<input
									type="number"
									min="0"
									max="1440"
									bind:value={timingDelayMinutes}
								/>
							</label>
							{#if timingDelayMinutes > 0}
								<p class="cu-warning">
									{i18n.t.jobs.blurNote}
								</p>
							{/if}
							<div class="cu-hook-actions">
								<button
									type="button"
									class="cu-btn primary"
									onclick={() => saveTiming(job)}
								>
									{i18n.t.common.save}
								</button>
								<button type="button" class="cu-btn" onclick={() => toggleTiming(job)}>{i18n.t.common.cancel}</button>
							</div>
						</div>
					{/if}

					{#if autoUnlockFor === job.name}
						<p class="cu-warning">
							{i18n.t.jobs.autoUnlockWarning}
						</p>
						<form
							class="cu-auto-unlock"
							onsubmit={(e) => {
								e.preventDefault();
								confirmAutoUnlock();
							}}
						>
							<input
								type="password"
								bind:value={autoUnlockPassword}
								placeholder={i18n.t.jobs.vaultPasswordPlaceholder}
								autocomplete="current-password"
							/>
							<button type="submit" class="cu-btn primary">{i18n.t.jobs.saveToKeychain}</button>
						</form>
					{/if}

					{#if historyFor === job.name}
						{#if historyRows.length === 0}
							<p class="cu-note">{i18n.t.jobs.noRuns}</p>
						{:else}
							<table class="cu-history">
								<tbody>
									{#each historyRows as row (row.runId)}
										<tr>
											<td class="cu-mono">{formatDate(row.startedAt)}</td>
											<td>{row.outcome ? i18n.t.jobs[outcomeKeys[row.outcome]] : i18n.t.jobs.outcomeRunning}</td>
											<td class="cu-mono">
												{row.filesUploaded}↑ {row.filesDeleted}✕ {row.filesRenamed}→ {formatBytes(row.bytesTransferred)}
											</td>
										</tr>
									{/each}
								</tbody>
							</table>
						{/if}
					{/if}

					{#if verifyFor === job.name}
						<div class="cu-excludes">
							<p class="cu-note">
								{i18n.t.jobs.verifyIntro}
							</p>
							<div class="cu-row-actions">
								<button
									type="button"
									class="cu-btn"
									disabled={verifyRunning || session.jobRunning}
									onclick={() => runVerify(job.name, 1)}
								>
									{i18n.t.jobs.verifyQuick}
								</button>
								<button
									type="button"
									class="cu-btn"
									disabled={verifyRunning || session.jobRunning}
									onclick={() => runVerify(job.name, 2)}
								>
									{i18n.t.jobs.verifyFull}
								</button>
							</div>
							{#if verifyRunning}
								<p class="cu-note" role="status">{i18n.t.jobs.verifyRunning}</p>
							{/if}
							{#if verifyError}
								<p class="cu-error" role="alert">
									{verifyError}
								</p>
							{/if}
							{#if verifyResult}
								{#if verifyResult.issues.length === 0}
									<p class="cu-note" role="status">
										{i18n.t.jobs.verifyOk}
									</p>
								{:else}
									<p class="cu-error" role="alert">
										{plural(verifyResult.issues.length, i18n.t.jobs.verifyIssues)}
									</p>
									<ul class="cu-exclude-folders">
										{#each verifyResult.issues as issue (issue.vaultPath)}
											<li><span class="cu-mono">{issue.vaultPath}</span> — {describeMessage(issue.problem)}</li>
										{/each}
									</ul>
									<p class="cu-note">
										{i18n.t.jobs.verifyNote}
									</p>
								{/if}
							{/if}
						</div>
					{/if}

					{#if excludesFor === job.name}
						<div class="cu-excludes">
							<p class="cu-note">
								{i18n.t.jobs.excludesNote}
							</p>
							{#if excludesFolders.length === 0}
								<p class="cu-note">{i18n.t.jobs.excludesEmpty}</p>
							{:else}
								<ul class="cu-exclude-folders">
									{#each excludesFolders as folder (folder)}
										<li>
											<label class="cu-checkbox">
												<input
													type="checkbox"
													checked={!isFolderExcluded(job, folder)}
													onchange={() => toggleFolderExcluded(job, folder)}
												/>
												<span class="cu-mono">{folder}/</span>
											</label>
										</li>
									{/each}
								</ul>
							{/if}
							<form
								class="cu-exclude-add"
								onsubmit={(e) => {
									e.preventDefault();
									addFreeExcludePattern(job);
								}}
							>
								<input
									type="text"
									class="cu-mono"
									bind:value={newExcludePattern}
									placeholder={i18n.t.jobs.excludePatternPlaceholder}
								/>
								<button
									type="submit"
									class="cu-btn primary"
								>
									{i18n.t.common.add}
								</button>
							</form>
							{#if job.excludes.length > 0}
								<ul class="cu-exclude-list">
									{#each job.excludes as pattern (pattern)}
										<li>
											<span class="cu-mono">{pattern}</span>
											<button type="button" class="cu-btn sm" onclick={() => removeExcludePattern(job, pattern)}>
												{i18n.t.common.removeEntry}
											</button>
										</li>
									{/each}
								</ul>
							{/if}
						</div>
					{/if}

					{#if scheduleFor === job.name}
						<div class="cu-schedule">
							<p class="cu-note">
								{i18n.t.jobs.windowsIntro}
							</p>
							{#if scheduleError}
								<p class="cu-error" role="alert">
									{scheduleError}
								</p>
							{/if}
							{#if job.scheduleWindows.length === 0}
								<p class="cu-note">{i18n.t.jobs.windowsEmpty}</p>
							{:else}
								<ul class="cu-schedule-list">
									{#each job.scheduleWindows as window, index (index)}
										<li>
											<span class="cu-mono">{formatScheduleWindow(window)}</span>
											<button
												type="button"
												class="cu-btn sm"
												onclick={() => removeScheduleWindow(job, index)}
											>
												{i18n.t.common.removeEntry}
											</button>
										</li>
									{/each}
								</ul>
							{/if}
							<form
								class="cu-schedule-add"
								onsubmit={(e) => {
									e.preventDefault();
									addScheduleWindow(job);
								}}
							>
								<div class="cu-schedule-days">
									{#each DAY_LABELS as label, i (label)}
										<label class="cu-checkbox">
											<input
												type="checkbox"
												bind:checked={newWindowDays[i]}
											/>
											<span>{label}</span>
										</label>
									{/each}
								</div>
								<div class="cu-schedule-times">
									<label class="cu-field">
										<span>{i18n.t.jobs.windowFrom}</span>
										<input
											type="time"
											bind:value={newWindowFrom}
										/>
									</label>
									<label class="cu-field">
										<span>{i18n.t.jobs.windowTo}</span>
										<input
											type="time"
											bind:value={newWindowTo}
										/>
									</label>
									<button
										type="submit"
										class="cu-btn primary"
									>
										{i18n.t.jobs.windowAdd}
									</button>
								</div>
							</form>
						</div>
					{/if}

					{#if hooksFor === job.name}
						<div class="cu-hooks">
							<p class="cu-note">
								{i18n.t.jobs.hooksIntro}
							</p>

							<div class="cu-hook-block">
								<h3>{i18n.t.jobs.preHook}</h3>
								<label class="cu-field">
									<span>{i18n.t.jobs.hookProgram}</span>
									<input
										type="text"
										class="cu-mono"
										bind:value={preProgram}
										placeholder={i18n.t.jobs.hookProgramPlaceholder}
									/>
								</label>
								<label class="cu-field">
									<span>{i18n.t.jobs.hookArgs}</span>
									<input
										type="text"
										class="cu-mono"
										bind:value={preArgs}
									/>
								</label>
								<label class="cu-field">
									<span>{i18n.t.jobs.hookTimeout}</span>
									<input
										type="number"
										min="1"
										bind:value={preTimeoutSecs}
									/>
								</label>
								<div class="cu-hook-actions">
									<button
										type="button"
										class="cu-btn primary"
										onclick={() => savePreHook(job)}
									>
										{i18n.t.jobs.savePreHook}
									</button>
									{#if job.preHook}
										<button type="button" class="cu-btn" onclick={() => clearPreHook(job)}>{i18n.t.common.removeEntry}</button>
									{/if}
								</div>
							</div>

							<div class="cu-hook-block">
								<h3>{i18n.t.jobs.postHook}</h3>
								<label class="cu-field">
									<span>{i18n.t.jobs.hookProgram}</span>
									<input
										type="text"
										class="cu-mono"
										bind:value={postProgram}
										placeholder={i18n.t.jobs.hookProgramPlaceholder}
									/>
								</label>
								<label class="cu-field">
									<span>{i18n.t.jobs.hookArgs}</span>
									<input
										type="text"
										class="cu-mono"
										bind:value={postArgs}
									/>
								</label>
								<label class="cu-field">
									<span>{i18n.t.jobs.hookTimeout}</span>
									<input
										type="number"
										min="1"
										bind:value={postTimeoutSecs}
									/>
								</label>
								<div class="cu-hook-actions">
									<button
										type="button"
										class="cu-btn primary"
										onclick={() => savePostHook(job)}
									>
										{i18n.t.jobs.savePostHook}
									</button>
									{#if job.postHook}
										<button type="button" class="cu-btn" onclick={() => clearPostHook(job)}>{i18n.t.common.removeEntry}</button>
									{/if}
								</div>
							</div>
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>

<style>
	.cu-page {
		padding: 30px 40px 26px;
	}

	.cu-toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 16px;
	}

	.cu-new-job {
		display: flex;
		flex-direction: column;
		gap: 12px;
		max-width: 420px;
		margin-bottom: 20px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		padding: 16px;
	}

	.cu-new-job .cu-btn {
		align-self: flex-start;
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
	}

	.cu-jobs {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.cu-jobs > li {
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		padding: 14px 16px;
		margin-bottom: 10px;
	}

	.cu-job-row {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 210px 170px;
		align-items: center;
		gap: 20px;
	}

	@media (max-width: 960px) {
		.cu-job-row {
			grid-template-columns: minmax(0, 1fr) auto;
			row-gap: 12px;
		}

		.cu-job-col-name {
			grid-column: 1 / -1;
		}
	}

	.cu-job-col-name {
		min-width: 0;
	}

	.cu-job-name {
		font-size: 20px;
		font-weight: 600;
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.cu-job-detail {
		color: var(--cu-muted);
		font-size: 12px;
		margin-top: 2px;
	}

	.cu-job-col-status {
		font-size: 13px;
	}

	.cu-job-status-line {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.cu-job-status-ok {
		color: var(--cu-verified);
	}

	.cu-job-status-warn {
		color: var(--cu-warn);
	}

	.cu-job-status-muted {
		color: var(--cu-muted);
	}

	.cu-job-status-sub {
		color: var(--cu-muted);
		font-size: 12px;
		margin-top: 3px;
	}

	.cu-job-bar {
		height: 2px;
		background: var(--cu-line-strong);
		margin-top: 6px;
		border-radius: 1px;
		overflow: hidden;
	}

	.cu-job-bar b {
		display: block;
		height: 100%;
		background: var(--cu-seal-text);
		transition: width var(--cu-motion-fast) var(--cu-ease-out);
	}

	.cu-job-col-actions {
		display: flex;
		align-items: center;
		justify-content: flex-end;
		gap: 6px;
		position: relative;
	}

	.cu-job-menu-wrap {
		position: relative;
	}

	.cu-job-menu {
		position: absolute;
		top: calc(100% + 4px);
		inset-inline-end: 0;
		z-index: 20;
		display: flex;
		flex-direction: column;
		min-width: 190px;
		background: var(--cu-surface-raised);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-md);
		box-shadow: var(--cu-shadow);
		padding: 6px;
		transform-origin: top right;
		animation: cu-menu-in var(--cu-motion-base) var(--cu-ease-out) both;
	}

	@keyframes cu-menu-in {
		from {
			opacity: 0;
			transform: scale(0.97) translateY(-2px);
		}
		to {
			opacity: 1;
			transform: none;
		}
	}

	.cu-job-menu button {
		background: none;
		border: none;
		border-radius: 6px;
		color: var(--cu-text);
		text-align: start;
		padding: 7px 10px;
		font-size: 13px;
		cursor: pointer;
	}

	.cu-job-menu button:hover {
		background: var(--cu-surface);
	}

	.cu-job-menu .cu-job-menu-danger {
		color: var(--cu-conflict);
	}

	.cu-job-menu-sep {
		height: 1px;
		background: var(--cu-line);
		margin: 5px 2px;
	}

	.cu-warning-actions {
		display: flex;
		gap: 8px;
		margin-top: 8px;
	}

	.cu-badge {
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--cu-muted);
		border: 1px solid var(--cu-line);
		border-radius: 3px;
		padding: 1px 5px;
	}

	.cu-badge-ok {
		color: var(--cu-verified);
		border-color: var(--cu-verified);
	}

	.cu-transfer {
		display: flex;
		justify-content: space-between;
		background: var(--cu-bg);
		border: 1px solid var(--cu-seal);
		border-radius: var(--cu-radius-sm);
		padding: 8px 12px;
		font-size: 12px;
		margin-top: 10px;
	}

	.cu-summary {
		font-size: 12px;
		color: var(--cu-verified);
		margin: 10px 0 0;
	}

	.cu-warning {
		background: color-mix(in srgb, var(--cu-conflict) 12%, transparent);
		border: 1px solid var(--cu-conflict);
		color: var(--cu-text);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 12px;
		line-height: 1.5;
		margin: 10px 0 0;
		max-width: 420px;
	}

	.cu-conflict-list {
		list-style: none;
		margin: 6px 0 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.cu-auto-unlock {
		display: flex;
		gap: 8px;
		margin-top: 10px;
		max-width: 320px;
	}

	.cu-auto-unlock input {
		flex: 1;
		min-width: 0;
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		color: var(--cu-text);
	}

	.cu-history {
		width: 100%;
		border-collapse: collapse;
		margin-top: 10px;
		font-size: 12px;
	}

	.cu-history td {
		padding: 5px 6px;
		border-bottom: 1px solid var(--cu-line);
		color: var(--cu-muted);
	}

	.cu-excludes {
		margin-top: 10px;
		padding-top: 10px;
		border-top: 1px solid var(--cu-line);
		max-width: 420px;
	}

	.cu-exclude-folders {
		list-style: none;
		margin: 8px 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.cu-exclude-add {
		display: flex;
		gap: 8px;
		margin-top: 6px;
	}

	.cu-exclude-add input {
		flex: 1;
		min-width: 0;
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		color: var(--cu-text);
	}

	.cu-exclude-list {
		list-style: none;
		margin: 10px 0 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: 12px;
	}

	.cu-exclude-list li {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 8px;
	}

	.cu-schedule {
		margin-top: 10px;
		padding-top: 10px;
		border-top: 1px solid var(--cu-line);
		max-width: 460px;
	}

	.cu-schedule-list {
		list-style: none;
		margin: 10px 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: 12px;
	}

	.cu-schedule-list li {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 8px;
	}

	.cu-schedule-add {
		display: flex;
		flex-direction: column;
		gap: 10px;
		margin-top: 6px;
	}

	.cu-schedule-days {
		display: flex;
		flex-wrap: wrap;
		gap: 10px;
	}

	.cu-schedule-days .cu-checkbox {
		display: flex;
		align-items: center;
		gap: 4px;
		font-size: 12px;
	}

	.cu-schedule-times {
		display: flex;
		align-items: flex-end;
		gap: 10px;
	}

	.cu-schedule-times input[type='time'] {
		width: auto;
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		color: var(--cu-text);
	}

	.cu-hooks {
		margin-top: 10px;
		padding-top: 10px;
		border-top: 1px solid var(--cu-line);
		max-width: 460px;
		display: flex;
		flex-direction: column;
		gap: 16px;
	}

	.cu-hook-block h3 {
		margin: 0 0 8px;
		font-size: 13px;
		font-weight: 600;
	}

	.cu-hook-block .cu-field {
		margin-bottom: 8px;
	}

	.cu-hook-block input[type='number'] {
		width: 100px;
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		color: var(--cu-text);
	}

	.cu-hook-actions {
		display: flex;
		gap: 8px;
	}

</style>
