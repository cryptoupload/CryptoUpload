import { lockNoticeTake, vaultLock, webviewRecycle } from '$lib/api/vault';
import type { ForceLockReport } from '$lib/api/security';
import { session } from './session.svelte';
import { fill, i18n, plural } from '$lib/i18n/index.svelte';
import { resetMount } from './mount.svelte';

function showLocked(): void {
	session.vaultInfo = null;
	session.pendingRemoveMemberResult = null;
	session.vaultRoot = '';
	session.section = 'vault';
	session.screen = 'loading';
	session.recoveryPhraseConfirmed = false;
	session.failedUnlocks = [];
	resetMount();
	setTimeout(() => {
		void webviewRecycle({ forcedLock: session.forcedLock, autoLocked: session.autoLockedNotice })
			.catch(() => false)
			.then((recycled) => {
				if (!recycled) session.screen = 'unlock';
			});
	}, 0);
}

export async function restoreLockNotice(): Promise<void> {
	const notice = await lockNoticeTake().catch(() => null);
	if (!notice) return;
	session.forcedLock = notice.forcedLock;
	session.autoLockedNotice = notice.autoLocked;
}

export async function lockVault(): Promise<void> {
	await vaultLock();
	showLocked();
}

export function applyForcedLock(report: ForceLockReport): void {
	if (!report.locked) return;
	session.forcedLock = report;
	showLocked();
}

export function forcedLockMessage(report: ForceLockReport): string {
	const t = i18n.t.lock;
	const parts = [
		report.reason === 'tripwire' && report.detail ? fill(t.tripwireProgram, { program: report.detail }) : t.reason[report.reason],
	];
	if (report.wasMounted) parts.push(t.driveUnmounted);
	if (report.unpublished > 0) parts.push(plural(report.unpublished, t.unpublished));
	if (report.cancelledJobs > 0) parts.push(t.jobCancelled);
	if (report.driveStillReadable) parts.push(t.stillReadable);
	return parts.join(' ');
}
