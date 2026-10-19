import { describeMessage } from '$lib/errors';
import type { ActivityKind } from '$lib/api/overview';
import { fill, i18n, plural } from '$lib/i18n/index.svelte';

export function activityText(a: ActivityKind): string {
	const t = i18n.t.activity;
	switch (a.kind) {
		case 'runFinished': {
			const parts = [];
			if (a.uploaded) parts.push(plural(a.uploaded, t.uploaded));
			if (a.renamed) parts.push(plural(a.renamed, t.renamed));
			if (a.deleted) parts.push(plural(a.deleted, t.deleted));
			return parts.length ? fill(t.runFinished, { job: a.job, details: parts.join(', ') }) : fill(t.runNothing, { job: a.job });
		}
		case 'runFailed':
			return fill(t.runFailed, { job: a.job, message: describeMessage(a.message) });
		case 'runCancelled':
			return fill(t.runCancelled, { job: a.job });
		case 'conflictKeptBoth':
			return fill(t.conflictKeptBoth, { path: a.path });
		case 'anomalyBlocked':
			return fill(t.anomalyBlocked, { job: a.job, reason: describeMessage(a.reason) });
		case 'verifyPassed':
			return plural(a.files, t.verifyPassed);
		case 'verifyIssues':
			return plural(a.issues, t.verifyIssues);
		case 'imported':
			return plural(a.files, t.imported, { path: a.path || i18n.t.nav.vault });
		case 'trashEmptied':
			return plural(a.items, t.trashEmptied);
	}
}
