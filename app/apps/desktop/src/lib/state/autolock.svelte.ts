import { mountStatus } from '$lib/api/mount';
import { securityRuntime, vaultAutoLock, vaultForceLock } from '$lib/api/security';
import { session } from './session.svelte';
import { applyForcedLock } from './lock';

const ACTIVITY_EVENTS = ['pointerdown', 'keydown', 'wheel', 'focus'] as const;
const CHECK_INTERVAL_MS = 5_000;
export const UNMOUNT_WARNING_MS = 30_000;
const SLEEP_GAP_MS = 60_000;

let minutes = 0;
let lastActivity = Date.now();
let lastTick = Date.now();
let intervalId: ReturnType<typeof setInterval> | null = null;
let locking = false;

export const autolock = $state<{ unmountAt: number | null }>({ unmountAt: null });

export function isAutoLockExpired(lastActivity: number, now: number, minutes: number): boolean {
	if (minutes <= 0) return false;
	return now - lastActivity >= minutes * 60_000;
}

export function effectiveIdleMinutes(userMinutes: number, capMinutes: number): number {
	if (capMinutes <= 0) return Math.max(userMinutes, 0);
	if (userMinutes <= 0) return capMinutes;
	return Math.min(userMinutes, capMinutes);
}

export function sleptBetween(previousTick: number, now: number): boolean {
	return now - previousTick >= SLEEP_GAP_MS;
}

function noteActivity() {
	lastActivity = Date.now();
	autolock.unmountAt = null;
}

export function keepUnlocked(): void {
	noteActivity();
}

async function tick() {
	const now = Date.now();
	const slept = sleptBetween(lastTick, now);
	lastTick = now;
	if (locking) return;
	if (session.screen !== 'unlocked') {
		autolock.unmountAt = null;
		return;
	}

	locking = true;
	try {
		const runtime = await securityRuntime();
		if (slept && runtime.lockOnSessionEvents) {
			applyForcedLock(await vaultForceLock('suspend'));
			return;
		}
		if (session.jobRunning) return;
		const limit = effectiveIdleMinutes(minutes, runtime.maxIdleMinutes);
		if (!isAutoLockExpired(lastActivity, now, limit)) {
			autolock.unmountAt = null;
			return;
		}

		const status = await mountStatus();
		if (status.mounted) {
			if (!runtime.lockUnmountsDrive) return;
			if (autolock.unmountAt === null) {
				autolock.unmountAt = now + UNMOUNT_WARNING_MS;
				return;
			}
			if (now < autolock.unmountAt) return;
		}

		const outcome = await vaultAutoLock();
		if (outcome.status !== 'locked' || !outcome.report.locked) return;
		autolock.unmountAt = null;
		applyForcedLock(outcome.report);
		if (!outcome.report.wasMounted && outcome.report.cancelledJobs === 0) {
			session.forcedLock = null;
			session.autoLockedNotice = true;
		}
	} catch {
	} finally {
		locking = false;
	}
}

export function initAutoLock(initialMinutes: number): void {
	minutes = initialMinutes;
	lastActivity = Date.now();
	lastTick = Date.now();
	if (intervalId !== null) return;
	for (const event of ACTIVITY_EVENTS) {
		window.addEventListener(event, noteActivity, { passive: true });
	}
	intervalId = setInterval(tick, CHECK_INTERVAL_MS);
}

export function setAutoLockMinutes(newMinutes: number): void {
	minutes = newMinutes;
	lastActivity = Date.now();
	autolock.unmountAt = null;
}
