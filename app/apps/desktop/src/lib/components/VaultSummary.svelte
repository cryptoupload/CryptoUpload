<script lang="ts">
	import { fill, i18n, plural } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import { session } from '$lib/state/session.svelte';
	import { mount, refreshMount, toggleMount as toggleMountState } from '$lib/state/mount.svelte';
	import { mountOpenFolder } from '$lib/api/mount';
	import { sync } from '$lib/state/sync.svelte';
	import { sceneMotionOff } from '$lib/state/theme.svelte';
	import { friendlyError } from '$lib/errors';
	import { unlockAttemptsTake } from '$lib/api/security';
	import { formatAgo, formatBytes, formatWhen } from '$lib/format';
	import { activityText } from '$lib/activityText';
	import { jobList, jobHistory, type Job, type RunRow } from '$lib/api/jobs';
	import { storageUsage, type JobUsage } from '$lib/api/usage';
	import { activityRecent, vaultRecent, type ActivityRecord, type RecentFile } from '$lib/api/overview';

	let { entering = false }: { entering?: boolean } = $props();

	let mountError = $state<string | null>(null);

	onMount(async () => {
		try {
			const attempts = await unlockAttemptsTake();
			if (attempts.length > 0) session.failedUnlocks = attempts;
		} catch {
		}
	});
	const failedUnlockText = $derived.by(() => {
		const attempts = session.failedUnlocks;
		if (attempts.length === 0) return null;
		const last = attempts[attempts.length - 1];
		return plural(attempts.length, i18n.t.home.failedUnlocks, { when: formatWhen(last.at) });
	});

	onMount(() => {
		const timer = setInterval(() => {
			if (mount.mounted && !mount.readOnly) void refreshMount();
		}, 2000);
		return () => clearInterval(timer);
	});
	const mounted = $derived(mount.mounted);
	const mountpoint = $derived(mount.mountpoint);

	async function toggleMount() {
		mountError = null;
		try {
			await toggleMountState();
		} catch (e) {
			mountError = friendlyError(e);
		}
	}

	async function openMountInFileManager() {
		if (!mountpoint) return;
		try {
			await mountOpenFolder();
		} catch (e) {
			mountError = friendlyError(e);
		}
	}

	let pathCopied = $state(false);

	async function copyMountPath() {
		if (!mountpoint) return;
		try {
			await navigator.clipboard.writeText(mountpoint);
			pathCopied = true;
			setTimeout(() => (pathCopied = false), 2000);
		} catch {
		}
	}

	let jobs = $state<Job[]>([]);
	let usage = $state<JobUsage[]>([]);
	let lastRuns = $state<Record<string, RunRow | undefined>>({});
	let recent = $state<RecentFile[]>([]);
	let activity = $state<ActivityRecord[]>([]);
	let loaded = $state(false);

	async function load() {
		const root = session.vaultRoot;
		if (!root) return;
		try {
			const [allJobs, u, r, a] = await Promise.all([
				jobList(),
				storageUsage().catch(() => [] as JobUsage[]),
				vaultRecent(root, 6).catch(() => [] as RecentFile[]),
				activityRecent(root, 6).catch(() => [] as ActivityRecord[]),
			]);
			jobs = allJobs.filter((j) => j.matchesOpenVault);
			const names = new Set(jobs.map((j) => j.name));
			usage = u.filter((x) => names.has(x.jobName));
			recent = r;
			activity = a;
			const rows = await Promise.all(jobs.map((j) => jobHistory(j.name, 1).catch(() => [] as RunRow[])));
			lastRuns = Object.fromEntries(jobs.map((j, i) => [j.name, rows[i][0]]));
		} catch {
		} finally {
			loaded = true;
		}
	}

	onMount(load);
	$effect(() => {
		void sync.version;
		load();
	});

	const totalFiles = $derived(usage.reduce((n, u) => n + u.fileCount, 0));
	const currentBytes = $derived(usage.reduce((n, u) => n + u.totalBytes, 0));
	const versionBytes = $derived(usage.reduce((n, u) => n + u.versionBytes, 0));
	const lastFinished = $derived(
		Object.values(lastRuns).reduce<number | null>((max, r) => {
			const t = r?.finishedAt ?? null;
			return t !== null && (max === null || t > max) ? t : max;
		}, null),
	);

	const vaultName = $derived(session.vaultRoot.split(/[/\\]/).filter(Boolean).pop() ?? session.vaultRoot);

	function openFolderOf(path: string) {
		session.browseDir = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
		session.section = 'browse';
	}

	function runState(name: string): { icon: string; text: string; cls: string } {
		if (sync.run?.name === name) return { icon: 'cu-i-sync', text: i18n.t.home.stateRunning, cls: 'sync' };
		const r = lastRuns[name];
		if (!r) return { icon: 'cu-i-clock', text: i18n.t.home.stateNever, cls: 'muted' };
		if (r.outcome === 'ok') return { icon: 'cu-i-check', text: i18n.t.home.stateUpToDate, cls: 'ok' };
		if (r.outcome === 'interrupted') return { icon: 'cu-i-pause', text: i18n.t.home.statePaused, cls: 'muted' };
		return { icon: 'cu-i-warn', text: i18n.t.home.stateCheck, cls: 'warn' };
	}

	let shownFiles = $state(0);
	let counted = false;
	$effect(() => {
		const target = totalFiles;
		if (!loaded) return;
		if (counted || !entering || sceneMotionOff() || target === 0) {
			shownFiles = target;
			return;
		}
		counted = true;
		const t0 = performance.now() + 650;
		const tick = (t: number) => {
			const k = Math.max(0, Math.min(1, (t - t0) / 1100));
			shownFiles = Math.round(target * (1 - Math.pow(1 - k, 3)));
			if (k < 1) requestAnimationFrame(tick);
		};
		requestAnimationFrame(tick);
	});
</script>

<div class="cu-page" class:cu-entering={entering}>
	{#if failedUnlockText}
		<div class="cu-failed-unlocks" role="alert">
			<span>{fill(i18n.t.home.failedUnlockNotice, { text: failedUnlockText })}</span>
			<button type="button" class="cu-btn sm" onclick={() => (session.failedUnlocks = [])}>
				{i18n.t.common.understood}
			</button>
		</div>
	{/if}
	<header class="cu-ohead">
		<div class="cu-ohead-text">
			<h1 class="cu-page-title">{vaultName}</h1>
			<p class="cu-ostat">
				{#if session.jobRunning}
					<svg class="cu-i cu-i-spin"><use href="#cu-i-sync" /></svg>
					{i18n.t.syncStrip.syncing}
				{:else}
					<svg class="cu-i"><use href="#cu-i-check" /></svg>
					{jobs.length ? i18n.t.home.sealedSynced : i18n.t.home.sealed}
					{#if jobs.length}{' — '}<span class="cu-num">{plural(shownFiles, i18n.t.home.fileCount)}</span>{#if lastFinished}{fill(i18n.t.home.lastRun, { ago: formatAgo(lastFinished / 1000) })}{/if}
					{/if}
				{/if}
			</p>
		</div>
		<div class="cu-ohead-btns">
			<button type="button" class="cu-btn primary" onclick={() => (session.section = 'browse')}>
				<svg class="cu-i"><use href="#cu-i-folder" /></svg>
				{i18n.t.home.openVault}
			</button>
			{#if mount.supported}
				{#if !mounted && mount.policyReadOnly}
					<label
						class="cu-mount-ro"
						title={i18n.t.home.allowWriteTitle}
					>
						<input type="checkbox" bind:checked={mount.confirmWrite} />
						<span>{i18n.t.home.allowWrite}</span>
					</label>
				{:else if !mounted}
					<label class="cu-mount-ro">
						<input type="checkbox" bind:checked={mount.readOnly} />
						<span>{i18n.t.home.readOnly}</span>
					</label>
				{/if}
				<button type="button" class="cu-btn" disabled={mount.busy} onclick={toggleMount}>
					<svg class="cu-i"><use href="#cu-i-drive" /></svg>
					{#if mount.busy}{i18n.t.common.working}{:else if mounted}{i18n.t.home.unmount}{:else}{i18n.t.home.mount}{/if}
				</button>
			{/if}
		</div>
	</header>

	<div class="cu-mount-block" hidden={!mounted && !mountError && mount.supported}>
		{#if mounted && mountpoint}
			<div class="cu-mount-row">
				<span class="cu-mount-label">{i18n.t.home.virtualDrive}</span>
				<span class="cu-mount-path cu-mono">{mountpoint}</span>
				<span class="cu-mount-mode">
					{#if mount.readOnly}{i18n.t.home.readOnly}{:else if mount.pendingFiles > 0}
						<svg class="cu-i cu-i-spin"><use href="#cu-i-sync" /></svg>
						{plural(mount.pendingFiles, i18n.t.home.filesUploading)}
					{:else}{i18n.t.home.writable}{/if}
				</span>
				<button type="button" class="cu-btn sm" onclick={openMountInFileManager}>
					{i18n.t.home.openInFileManager}
				</button>
				<button type="button" class="cu-btn sm" onclick={copyMountPath}>
					{pathCopied ? i18n.t.common.copied : i18n.t.home.copyPath}
				</button>
			</div>
		{:else if !mount.supported}
			<p class="cu-note">{i18n.t.home.mountUnsupported}</p>
		{/if}
		{#if mountError}
			<p class="cu-error" role="alert">{mountError}</p>
		{/if}
	</div>

	{#if session.vaultInfo?.hasRecoveryPhrase && !session.recoveryPhraseConfirmed}
		<p class="cu-note cu-warning">
			<svg class="cu-i"><use href="#cu-i-warn" /></svg>
			{i18n.t.home.recoveryUnconfirmed}
		</p>
	{/if}

	<div class="cu-ogrid">
		<section class="cu-opanel cu-rise" style="--i:0">
			<h2>{i18n.t.home.recentFiles}</h2>
			{#if recent.length}
				<ul class="cu-mini">
					{#each recent as f (f.job + f.path)}
						<li>
							<button type="button" class="cu-mini-row" title={f.path} onclick={() => openFolderOf(f.path)}>
								<svg class="cu-i"><use href="#cu-i-file" /></svg>
								<span class="cu-mini-text">
									<span class="cu-mini-name">{f.path.split('/').pop()}</span>
									<span class="cu-mini-meta cu-num">{formatBytes(f.size)} · {formatWhen(f.modified)}</span>
								</span>
							</button>
						</li>
					{/each}
				</ul>
			{:else if loaded}
				<p class="cu-empty-note">
					{jobs.length
						? i18n.t.home.recentAfterFirstRun
						: i18n.t.home.recentNoJobs}
				</p>
			{:else}
				<p class="cu-empty-note">{i18n.t.common.loading}</p>
			{/if}
		</section>

		<section class="cu-opanel cu-rise" style="--i:1">
			<h2>{i18n.t.home.spaceAndJobs}</h2>
			<div class="cu-num">{formatBytes(currentBytes + versionBytes)} <span class="cu-muted">{i18n.t.home.used}</span></div>
			{#if currentBytes + versionBytes > 0}
				<div
					class="cu-meter"
					role="img"
					aria-label={fill(i18n.t.home.meterLabel, { current: formatBytes(currentBytes), versions: formatBytes(versionBytes) })}
				>
					<i class="cur" style:width={`${(currentBytes / (currentBytes + versionBytes)) * 100}%`}></i>
					<i class="old" style:width={`${(versionBytes / (currentBytes + versionBytes)) * 100}%`}></i>
				</div>
				<p class="cu-legend">
					<span><i class="cur"></i>{fill(i18n.t.home.currentFiles, { size: formatBytes(currentBytes) })}</span>
					<span><i class="old"></i>{fill(i18n.t.home.earlierVersions, { size: formatBytes(versionBytes) })}</span>
				</p>
			{/if}
			{#if jobs.length}
				<ul class="cu-prov">
					{#each jobs as j (j.name)}
						{@const st = runState(j.name)}
						<li>
							<span class="cu-prov-name">{j.name}</span>
							<span class="cu-st cu-st-{st.cls}"><svg class="cu-i"><use href="#{st.icon}" /></svg>{st.text}</span>
						</li>
					{/each}
				</ul>
			{:else if loaded}
				<p class="cu-empty-note">
					<button type="button" class="cu-text-link" onclick={() => (session.section = 'jobs')}>{i18n.t.syncStrip.createJob}</button>
					{i18n.t.home.createJobRest}
				</p>
			{:else}
				<p class="cu-empty-note">{i18n.t.common.loading}</p>
			{/if}
		</section>
	</div>

	<section class="cu-rise" style="--i:2">
		<h2>{i18n.t.home.recentActivity}</h2>
		{#if activity.length}
			<ol class="cu-log">
				{#each activity as a, i (a.at + ':' + i)}
					<li class:warn={a.warning}>
						<time class="cu-mono">{formatWhen(a.at)}</time>
						<span>{activityText(a)}</span>
					</li>
				{/each}
			</ol>
		{:else if loaded}
			<p class="cu-empty-note">{i18n.t.home.activityEmpty}</p>
		{:else}
			<p class="cu-empty-note">{i18n.t.common.loading}</p>
		{/if}
	</section>

	{#if session.vaultInfo}
		{@const info = session.vaultInfo}
		<details class="cu-tech-details">
			<summary>{i18n.t.home.technical}</summary>
			<dl class="cu-facts">
				<div><dt>{i18n.t.home.techPath}</dt><dd class="cu-mono">{session.vaultRoot}</dd></div>
				<div>
					<dt>{i18n.t.home.techProfile}</dt>
					<dd>{info.privateProfile
							? info.decoyFiles
								? i18n.t.home.profilePrivateDecoys
								: i18n.t.home.profilePrivate
							: i18n.t.home.profileBase}</dd>
				</div>
				<div><dt>{i18n.t.home.techFileFormat}</dt><dd class="cu-mono">{info.fileFormat}</dd></div>
				<div><dt>{i18n.t.home.techNameFormat}</dt><dd class="cu-mono">{info.nameFormat}</dd></div>
				<div><dt>{i18n.t.home.techKdf}</dt><dd class="cu-mono">{info.kdfName}</dd></div>
				<div><dt>{i18n.t.home.techSeed}</dt><dd>{info.seedCount}</dd></div>
				<div>
					<dt>{i18n.t.home.techRecovery}</dt>
					<dd class:verified={info.hasRecoveryPhrase}>{info.hasRecoveryPhrase ? i18n.t.home.recoveryActive : i18n.t.home.recoveryNotSet}</dd>
				</div>
			</dl>
		</details>
	{/if}
</div>

<style>
	.cu-failed-unlocks {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
		margin: 0 0 16px;
		padding: 10px 14px;
		border: 1px solid var(--cu-warn);
		border-radius: var(--cu-radius-sm);
		font-size: 13px;
	}

	.cu-page {
		padding: 30px 40px 26px;
	}

	.cu-ohead {
		display: flex;
		justify-content: space-between;
		align-items: flex-end;
		gap: 24px;
		flex-wrap: wrap;
		margin-bottom: 24px;
		padding-bottom: 18px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-ohead-text {
		min-width: 0;
	}

	.cu-ohead h1 {
		word-break: break-word;
	}

	.cu-ohead-btns {
		display: flex;
		gap: 8px;
		flex-wrap: wrap;
	}

	.cu-ostat {
		display: flex;
		align-items: center;
		gap: 8px;
		margin: 8px 0 0;
		font-size: 13px;
		color: var(--cu-muted);
	}

	.cu-ostat .cu-i {
		color: var(--cu-verified);
	}

	.cu-ostat .cu-i-spin {
		color: var(--cu-seal-text);
		animation: cu-spin 1.6s linear infinite;
	}

	@keyframes cu-spin {
		to {
			transform: rotate(360deg);
		}
	}

	.cu-mount-block {
		margin: -8px 0 20px;
	}

	.cu-mount-block[hidden] {
		display: none;
	}

	.cu-mount-row {
		display: flex;
		align-items: center;
		gap: 10px;
		flex-wrap: wrap;
		padding: 10px 14px;
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		background: var(--cu-surface);
		font-size: 13px;
	}

	.cu-mount-label {
		color: var(--cu-muted);
	}

	.cu-mount-ro {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: 13px;
		color: var(--cu-muted);
	}

	.cu-mount-ro input {
		width: auto;
	}

	.cu-mount-mode {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		color: var(--cu-muted);
	}

	.cu-mount-path {
		flex: 1;
		min-width: 120px;
		word-break: break-all;
	}

	.cu-ogrid {
		display: grid;
		grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr);
		gap: 20px;
		margin-bottom: 26px;
	}

	@media (max-width: 860px) {
		.cu-ogrid {
			grid-template-columns: minmax(0, 1fr);
		}
	}

	.cu-opanel {
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		padding: 18px 20px;
		min-width: 0;
	}

	h2 {
		margin: 0 0 8px;
		font-size: 16px;
		line-height: 1.25;
		font-weight: 600;
		color: var(--cu-muted);
		letter-spacing: -0.01em;
	}

	.cu-mini {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.cu-mini li + li {
		border-top: 1px solid var(--cu-line);
	}

	.cu-mini-row {
		display: flex;
		align-items: center;
		gap: 12px;
		width: calc(100% + 12px);
		min-height: 48px;
		padding: 4px 6px;
		margin: 0 -6px;
		background: none;
		border: 0;
		border-radius: var(--cu-radius-sm);
		color: var(--cu-text);
		text-align: start;
		cursor: pointer;
	}

	.cu-mini-row:hover {
		background: var(--cu-surface);
	}

	.cu-mini-row .cu-i {
		color: var(--cu-muted);
	}

	.cu-mini-text {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}

	.cu-mini-name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.cu-mini-meta {
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-muted {
		color: var(--cu-muted);
	}

	.cu-meter {
		display: flex;
		height: 6px;
		margin: 10px 0 8px;
		border-radius: 6px;
		overflow: hidden;
		background: var(--cu-line);
	}

	.cu-meter i,
	.cu-legend i {
		display: block;
		height: 100%;
	}

	.cur {
		background: var(--cu-seal-text);
	}

	.old {
		background: var(--cu-muted);
		opacity: 0.6;
	}

	.cu-legend {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 16px;
		margin: 0 0 8px;
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-legend span {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}

	.cu-legend i {
		width: 8px;
		height: 8px;
		border-radius: 2px;
	}

	.cu-prov {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.cu-prov li {
		display: flex;
		justify-content: space-between;
		gap: 12px;
		padding: 9px 0;
		border-bottom: 1px solid var(--cu-line);
		font-size: 13px;
	}

	.cu-prov li:last-child {
		border-bottom: 0;
	}

	.cu-prov-name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.cu-st {
		display: inline-flex;
		align-items: center;
		gap: 7px;
		white-space: nowrap;
		color: var(--cu-muted);
	}

	.cu-st .cu-i {
		width: 16px;
		height: 16px;
	}

	.cu-st-ok .cu-i {
		color: var(--cu-verified);
	}

	.cu-st-sync {
		color: var(--cu-text);
	}

	.cu-st-sync .cu-i {
		color: var(--cu-seal-text);
	}

	.cu-st-warn {
		color: var(--cu-warn);
	}

	.cu-empty-note {
		margin: 6px 0 0;
		font-size: 13px;
		color: var(--cu-muted);
	}

	.cu-log {
		list-style: none;
		margin: 12px 0 0;
		padding: 0 0 0 18px;
		border-inline-start: 1px solid var(--cu-line-strong);
	}

	.cu-log li {
		position: relative;
		padding: 0 0 16px;
		font-size: 13px;
	}

	.cu-log li:last-child {
		padding-bottom: 0;
	}

	.cu-log li::before {
		content: '';
		position: absolute;
		inset-inline-start: -23px;
		top: 6px;
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--cu-bg);
		border: 1px solid var(--cu-muted);
	}

	.cu-log li.warn::before {
		background: var(--cu-warn);
		border-color: var(--cu-warn);
	}

	.cu-log time {
		display: block;
		font-size: 12px;
		color: var(--cu-muted);
		font-variant-numeric: tabular-nums;
	}

	.cu-note {
		color: var(--cu-muted);
		font-size: 13px;
		margin: 0 0 16px;
	}

	.cu-warning {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		color: var(--cu-warn);
	}

	.cu-warning .cu-i {
		flex-shrink: 0;
		margin-top: 1px;
	}

	.cu-error {
		background: color-mix(in srgb, var(--cu-conflict) 15%, transparent);
		border: 1px solid var(--cu-conflict);
		color: var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 8px 0 0;
	}

	.cu-tech-details {
		margin: 30px 0 0;
	}

	.cu-tech-details summary {
		cursor: pointer;
		font-size: 12px;
		color: var(--cu-muted);
		list-style: none;
	}

	.cu-tech-details summary::-webkit-details-marker {
		display: none;
	}

	.cu-tech-details summary::before {
		content: '▸ ';
	}

	.cu-tech-details[open] summary::before {
		content: '▾ ';
	}

	.cu-tech-details summary:hover {
		color: var(--cu-text);
	}

	.cu-facts {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
		gap: 14px 24px;
		margin: 10px 0 0;
		padding: 18px 20px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
	}

	.cu-facts dt {
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--cu-muted);
		margin-bottom: 2px;
	}

	.cu-facts dd {
		margin: 0;
		font-size: 13px;
		word-break: break-all;
	}

	.cu-facts dd.verified {
		color: var(--cu-verified);
	}

	.cu-entering .cu-rise {
		animation: cu-rise 0.7s cubic-bezier(0.2, 0.7, 0.2, 1) both;
		animation-delay: calc(0.65s + var(--i, 0) * 90ms);
	}

	@keyframes cu-rise {
		from {
			opacity: 0;
			transform: translateY(12px);
		}
		to {
			opacity: 1;
			transform: none;
		}
	}
</style>
