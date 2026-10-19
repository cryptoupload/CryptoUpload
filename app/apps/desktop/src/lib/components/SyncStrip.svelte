<script lang="ts">
	import { fill, i18n, plural } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import { jobList, type Job } from '$lib/api/jobs';
	import { activityRecent, type ActivityRecord } from '$lib/api/overview';
	import { storageUsage } from '$lib/api/usage';
	import { isOfflineError, describeMessage } from '$lib/errors';
	import { formatAgo, formatBytes, formatSpeed } from '$lib/format';
	import { session } from '$lib/state/session.svelte';
	import { sync, startRun, cancelRun } from '$lib/state/sync.svelte';

	type Kind = 'syncing' | 'synced' | 'paused' | 'offline' | 'error' | 'conflict' | 'anomaly' | 'idle' | 'none';

	let jobs = $state<Job[]>([]);
	let activity = $state<ActivityRecord[]>([]);
	let fileCount = $state(0);
	let loaded = $state(false);

	async function refresh() {
		const root = session.vaultRoot;
		if (!root) return;
		try {
			const [j, a, u] = await Promise.all([jobList(), activityRecent(root, 30), storageUsage().catch(() => [])]);
			jobs = j.filter((job) => job.matchesOpenVault);
			activity = a;
			const names = new Set(jobs.map((job) => job.name));
			fileCount = u.filter((x) => names.has(x.jobName)).reduce((n, x) => n + x.fileCount, 0);
		} catch {
		} finally {
			loaded = true;
		}
	}

	onMount(refresh);
	$effect(() => {
		void sync.version;
		refresh();
	});

	let now = $state(Date.now());
	onMount(() => {
		const t = setInterval(() => (now = Date.now()), 30_000);
		return () => clearInterval(t);
	});

	const lastRunEvent = $derived(
		activity.find((a) => a.kind === 'runFinished' || a.kind === 'runFailed' || a.kind === 'runCancelled' || a.kind === 'anomalyBlocked'),
	);
	const lastConflict = $derived(activity.find((a) => a.kind === 'conflictKeptBoth'));

	const view = $derived.by((): { kind: Kind; title: string; detail: string; action: string | null; progress: number | null } => {
		const run = sync.run;
		if (run) {
			const remaining = Math.max(0, run.total - run.index + (run.index > 0 ? 1 : 0));
			const done = run.doneBytes + run.fileBytes;
			const parts = [plural(remaining, i18n.t.syncStrip.remaining)];
			if (run.totalBytes > 0) parts.push(fill(i18n.t.syncStrip.bytesOf, { done: formatBytes(done), total: formatBytes(run.totalBytes) }));
			if (run.speed > 0) parts.push(formatSpeed(run.speed));
			const progress = run.totalBytes > 0 ? done / run.totalBytes : run.total > 0 ? run.index / run.total : null;
			return {
				kind: 'syncing',
				title: run.cancelling ? i18n.t.syncStrip.pausing : i18n.t.syncStrip.syncing,
				detail: run.cancelling ? i18n.t.syncStrip.pausingDetail : parts.join(', '),
				action: run.cancelling ? null : i18n.t.syncStrip.pause,
				progress: Math.min(1, progress ?? 0),
			};
		}
		if (!loaded) return { kind: 'none', title: '', detail: '', action: null, progress: null };
		if (jobs.length === 0) {
			return {
				kind: 'idle',
				title: i18n.t.syncStrip.noFolder,
				detail: i18n.t.syncStrip.noFolderDetail,
				action: i18n.t.syncStrip.createJob,
				progress: null,
			};
		}
		const last = sync.last;
		const lastKind = last?.kind ?? (lastRunEvent ? { runFinished: 'completed', runFailed: 'failed', runCancelled: 'cancelled', anomalyBlocked: 'anomaly' }[lastRunEvent.kind as string] : null);
		const lastAt = last?.at ?? lastRunEvent?.at ?? null;
		if (lastKind === 'cancelled') {
			return { kind: 'paused', title: i18n.t.syncStrip.paused, detail: i18n.t.syncStrip.pausedDetail, action: i18n.t.syncStrip.resume, progress: null };
		}
		if (lastKind === 'anomaly') {
			return { kind: 'anomaly', title: i18n.t.syncStrip.stopped, detail: i18n.t.syncStrip.stoppedDetail, action: i18n.t.syncStrip.review, progress: null };
		}
		if (lastKind === 'failed') {
			const message = last?.text ?? (lastRunEvent?.kind === 'runFailed' ? lastRunEvent.message : '');
			if (last?.offline ?? isOfflineError(message)) {
				return { kind: 'offline', title: i18n.t.syncStrip.offline, detail: i18n.t.syncStrip.offlineDetail, action: i18n.t.common.retry, progress: null };
			}
			return { kind: 'error', title: i18n.t.syncStrip.failed, detail: describeMessage(message), action: i18n.t.common.retry, progress: null };
		}
		const conflicts = last ? last.conflicts.length : lastRunEvent?.kind === 'runFinished' ? lastRunEvent.conflicts : 0;
		if (lastKind === 'completed' && conflicts > 0) {
			const path = last?.conflicts[0]?.vaultPath ?? (lastConflict?.kind === 'conflictKeptBoth' ? lastConflict.path : '');
			return {
				kind: 'conflict',
				title: i18n.t.syncStrip.keptBoth,
				detail: path ? fill(i18n.t.syncStrip.keptBothOne, { path }) : plural(conflicts, i18n.t.syncStrip.keptBothMany),
				action: i18n.t.syncStrip.seeFiles,
				progress: null,
			};
		}
		if (lastKind === 'completed' && lastAt) {
			return {
				kind: 'synced',
				title: i18n.t.syncStrip.synced,
				detail: plural(fileCount, i18n.t.syncStrip.syncedDetail, { ago: formatAgo(lastAt, now) }),
				action: i18n.t.syncStrip.syncNow,
				progress: 1,
			};
		}
		return { kind: 'idle', title: i18n.t.syncStrip.noRunYet, detail: plural(jobs.length, i18n.t.syncStrip.jobsReady), action: i18n.t.syncStrip.syncNow, progress: null };
	});

	const icon = $derived(
		{
			syncing: 'cu-i-sync',
			synced: 'cu-i-check',
			paused: 'cu-i-pause',
			offline: 'cu-i-off',
			error: 'cu-i-x',
			conflict: 'cu-i-copy',
			anomaly: 'cu-i-warn',
			idle: 'cu-i-cloud',
			none: 'cu-i-cloud',
		}[view.kind],
	);

	let flashKey = $state(0);
	let prevKind: Kind | null = null;
	$effect(() => {
		const k = view.kind;
		if (prevKind !== null && prevKind !== k) flashKey += 1;
		prevKind = k;
	});

	async function runAll() {
		for (const job of jobs.filter((j) => j.enabled)) {
			const outcome = await startRun(job.name);
			if (!outcome || outcome.kind !== 'completed') break;
		}
	}

	function act() {
		switch (view.kind) {
			case 'syncing':
				cancelRun();
				break;
			case 'paused':
			case 'offline':
			case 'error': {
				const name = sync.last?.name ?? (lastRunEvent && 'job' in lastRunEvent ? lastRunEvent.job : null);
				if (name) startRun(name);
				else runAll();
				break;
			}
			case 'conflict': {
				const path = sync.last?.conflicts[0]?.vaultPath ?? (lastConflict?.kind === 'conflictKeptBoth' ? lastConflict.path : '');
				session.browseDir = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
				session.section = 'browse';
				break;
			}
			case 'anomaly':
				session.section = 'jobs';
				break;
			case 'idle':
				if (jobs.length === 0) session.section = 'jobs';
				else runAll();
				break;
			default:
				runAll();
		}
	}
</script>

<div class="cu-strip" data-kind={view.kind} role="status" aria-live="polite">
	{#key flashKey}<span class="cu-strip-sweep" aria-hidden="true"></span>{/key}
	<span class="cu-strip-lamp"><svg class="cu-i"><use href="#{icon}" /></svg></span>
	<div class="cu-strip-txt">
		<span class="cu-strip-title">{view.title}</span>
		<span class="cu-strip-detail" title={view.detail}>{view.detail}</span>
	</div>
	<div class="cu-strip-track" aria-hidden="true">
		{#if view.progress !== null}
			<div class="cu-strip-fill" style:width={`${Math.round(view.progress * 100)}%`}></div>
		{/if}
	</div>
	{#if view.action}
		<button type="button" class="cu-btn sm" onclick={act}>{view.action}</button>
	{:else}
		<span></span>
	{/if}
</div>

<style>
	.cu-strip {
		position: relative;
		display: grid;
		grid-template-columns: auto minmax(0, 1fr) minmax(80px, 220px) auto;
		align-items: center;
		gap: 14px;
		min-height: 44px;
		padding: 0 16px;
		border-top: 1px solid var(--cu-line);
		background: var(--cu-rail);
		font-size: 13px;
	}

	.cu-strip-sweep {
		position: absolute;
		top: -1px;
		left: 0;
		height: 1px;
		width: 30%;
		background: linear-gradient(90deg, transparent, var(--cu-seal-text), transparent);
		opacity: 0;
		animation: cu-sweep 1.1s ease-out;
		pointer-events: none;
	}

	@keyframes cu-sweep {
		0% {
			opacity: 1;
			transform: translateX(-100%);
		}
		100% {
			opacity: 0;
			transform: translateX(370%);
		}
	}

	.cu-strip-lamp {
		display: grid;
		color: var(--cu-seal-text);
	}

	.cu-strip[data-kind='syncing'] .cu-strip-lamp {
		animation: cu-lamp-pulse 1.6s ease-in-out infinite;
	}

	.cu-strip[data-kind='synced'] .cu-strip-lamp {
		color: var(--cu-verified);
	}

	.cu-strip[data-kind='paused'] .cu-strip-lamp,
	.cu-strip[data-kind='offline'] .cu-strip-lamp,
	.cu-strip[data-kind='idle'] .cu-strip-lamp {
		color: var(--cu-muted);
	}

	.cu-strip[data-kind='error'] .cu-strip-lamp {
		color: var(--cu-conflict);
	}

	.cu-strip[data-kind='conflict'] .cu-strip-lamp,
	.cu-strip[data-kind='anomaly'] .cu-strip-lamp {
		color: var(--cu-warn);
	}

	@keyframes cu-lamp-pulse {
		0%,
		100% {
			opacity: 0.55;
		}
		50% {
			opacity: 1;
		}
	}

	.cu-strip-txt {
		display: flex;
		align-items: baseline;
		gap: 12px;
		min-width: 0;
	}

	.cu-strip-title {
		font-weight: 500;
		white-space: nowrap;
	}

	.cu-strip-detail {
		color: var(--cu-muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		min-width: 0;
		font-variant-numeric: tabular-nums;
	}

	.cu-strip-track {
		position: relative;
		height: 3px;
		border-radius: 3px;
		background: var(--cu-line-strong);
		overflow: hidden;
	}

	.cu-strip-fill {
		position: absolute;
		inset: 0 auto 0 0;
		border-radius: 3px;
		background: var(--cu-seal-text);
		transition: width 400ms var(--cu-ease-out);
	}

	.cu-strip[data-kind='synced'] .cu-strip-fill {
		background: var(--cu-verified);
	}

	.cu-strip[data-kind='syncing'] .cu-strip-fill::after {
		content: '';
		position: absolute;
		top: 0;
		bottom: 0;
		width: 36px;
		background: linear-gradient(90deg, transparent, rgb(255 255 255 / 0.7), transparent);
		animation: cu-scan 1.6s cubic-bezier(0.4, 0, 0.2, 1) infinite;
	}

	@keyframes cu-scan {
		from {
			transform: translateX(-40px);
		}
		to {
			transform: translateX(320px);
		}
	}
</style>
