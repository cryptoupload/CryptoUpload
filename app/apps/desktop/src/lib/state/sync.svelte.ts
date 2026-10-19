import { fill, i18n, plural } from '$lib/i18n/index.svelte';
import { jobRunNow, type ConflictInfo, type JobRunNowOutcome, type PostHookOutcome } from '$lib/api/jobs';
import { jobCancel } from '$lib/api/overview';
import { friendlyError, isOfflineError } from '$lib/errors';
import { formatBytes } from '$lib/format';
import { notify } from '$lib/notify';
import { session } from './session.svelte';

export interface ActiveRun {
	name: string;
	file: string;
	index: number;
	total: number;
	fileBytes: number;
	doneBytes: number;
	totalBytes: number;
	speed: number;
	cancelling: boolean;
}

export interface FinishedRun {
	name: string;
	kind: 'completed' | 'failed' | 'cancelled' | 'anomaly';
	text: string;
	offline?: boolean;
	conflicts: ConflictInfo[];
	postHookText: string | null;
	at: number;
}

export const sync = $state<{ run: ActiveRun | null; last: FinishedRun | null; version: number }>({
	run: null,
	last: null,
	version: 0,
});

function formatPostHookOutcome(outcome: PostHookOutcome | null): string | null {
	if (!outcome) return null;
	switch (outcome.kind) {
		case 'ok':
			return i18n.t.sync.postHookOk;
		case 'failed':
			return fill(i18n.t.sync.postHookFailed, { code: outcome.exitCode ?? '?' });
		case 'timedOut':
			return i18n.t.sync.postHookTimedOut;
		case 'spawnFailed':
			return fill(i18n.t.sync.postHookSpawnFailed, { reason: outcome.reason });
	}
}

const nowSecs = () => Math.floor(Date.now() / 1000);

export async function startRun(name: string, confirmAnomaly = false): Promise<JobRunNowOutcome | null> {
	if (sync.run) return null;
	sync.last = null;
	sync.run = {
		name,
		file: '',
		index: 0,
		total: 0,
		fileBytes: 0,
		doneBytes: 0,
		totalBytes: 0,
		speed: 0,
		cancelling: false,
	};
	session.jobRunning = true;
	let lastOp = '';
	let sampleAt = performance.now();
	let sampleBytes = 0;
	try {
		const outcome = await jobRunNow(
			name,
			(event) => {
				const run = sync.run;
				if (!run) return;
				if (event.kind === 'plan') {
					run.total = event.items;
					run.totalBytes = event.uploadBytes;
				} else if (event.kind === 'item') {
					if (lastOp === 'upload') run.doneBytes += run.fileBytes;
					lastOp = event.op;
					run.file = event.vaultPath;
					run.index = event.index;
					run.total = event.total;
					run.fileBytes = 0;
				} else {
					run.fileBytes = event.count;
				}
				const t = performance.now();
				if (t - sampleAt >= 500) {
					const moved = run.doneBytes + run.fileBytes - sampleBytes;
					const instant = moved > 0 ? (moved * 1000) / (t - sampleAt) : 0;
					run.speed = run.speed ? run.speed * 0.6 + instant * 0.4 : instant;
					sampleAt = t;
					sampleBytes = run.doneBytes + run.fileBytes;
				}
			},
			confirmAnomaly,
		);
		if (outcome.kind === 'anomalyBlocked') {
			const affected = { uploads: outcome.uploadsAffected, deletes: outcome.deletesAffected, renames: outcome.renamesAffected };
			sync.last = {
				name,
				kind: 'anomaly',
				text: fill(i18n.t.sync.anomalyText, affected),
				conflicts: [],
				postHookText: null,
				at: nowSecs(),
			};
			notify(
				fill(i18n.t.sync.notifyBlockedTitle, { name }),
				fill(i18n.t.sync.notifyBlockedBody, affected),
			);
		} else if (outcome.kind === 'cancelled') {
			sync.last = {
				name,
				kind: 'cancelled',
				text: i18n.t.sync.cancelledText,
				conflicts: [],
				postHookText: null,
				at: nowSecs(),
			};
		} else {
			const n = outcome.conflicts.length;
			const text =
				fill(i18n.t.sync.completedText, { uploads: outcome.uploads, renames: outcome.renames, deletes: outcome.deletes, bytes: formatBytes(outcome.bytesTransferred) }) +
				(n ? plural(n, i18n.t.sync.conflictsResolved) : '');
			sync.last = {
				name,
				kind: 'completed',
				text,
				conflicts: outcome.conflicts,
				postHookText: formatPostHookOutcome(outcome.postHook),
				at: nowSecs(),
			};
			notify(fill(i18n.t.sync.notifyDoneTitle, { name }), text);
		}
		return outcome;
	} catch (e) {
		const message = friendlyError(e);
		sync.last = { name, kind: 'failed', text: message, offline: isOfflineError(String(e)), conflicts: [], postHookText: null, at: nowSecs() };
		notify(fill(i18n.t.sync.notifyFailedTitle, { name }), message);
		return null;
	} finally {
		sync.run = null;
		sync.version += 1;
		session.jobRunning = false;
	}
}

export async function cancelRun(): Promise<void> {
	const run = sync.run;
	if (!run || run.cancelling) return;
	run.cancelling = true;
	try {
		await jobCancel(run.name);
	} catch {
		run.cancelling = false;
	}
}
