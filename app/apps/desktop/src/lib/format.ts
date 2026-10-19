import { fill, i18n } from '$lib/i18n/index.svelte';

export function formatBytes(n: number): string {
	if (n < 1024) return `${n} B`;
	if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
	if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
	return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

function sameDay(a: Date, b: Date): boolean {
	return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

function hhmm(d: Date): string {
	return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

export function formatWhen(unixSecs: number, now: Date = new Date()): string {
	const d = new Date(unixSecs * 1000);
	if (sameDay(d, now)) return fill(i18n.t.format.today, { time: hhmm(d) });
	const yesterday = new Date(now);
	yesterday.setDate(now.getDate() - 1);
	if (sameDay(d, yesterday)) return fill(i18n.t.format.yesterday, { time: hhmm(d) });
	return new Intl.DateTimeFormat(i18n.language.tag, { day: 'numeric', month: 'short', year: 'numeric' }).format(d);
}

function relative(value: number, unit: 'minute' | 'hour' | 'day'): string {
	return new Intl.RelativeTimeFormat(i18n.language.tag, { numeric: 'always' }).format(value, unit);
}

export function formatAgo(unixSecs: number, nowMs: number = Date.now()): string {
	const s = Math.max(0, Math.round(nowMs / 1000 - unixSecs));
	if (s < 60) return i18n.t.format.now;
	const m = Math.floor(s / 60);
	if (m < 60) return relative(-m, 'minute');
	const h = Math.floor(m / 60);
	if (h < 24) return relative(-h, 'hour');
	return relative(-Math.floor(h / 24), 'day');
}

export function formatIn(unixSecs: number, nowMs: number = Date.now()): string {
	const s = Math.round(unixSecs - nowMs / 1000);
	if (s < 60) return i18n.t.format.soon;
	const m = Math.round(s / 60);
	if (m < 60) return relative(m, 'minute');
	return relative(Math.round(m / 60), 'hour');
}

export function formatSpeed(bytesPerSec: number): string {
	return `${formatBytes(Math.max(0, Math.round(bytesPerSec)))}/s`;
}

export function uniqueName(name: string, taken: ReadonlySet<string>, isDir = false): string {
	if (!taken.has(name.toLowerCase())) return name;
	const dot = isDir ? -1 : name.lastIndexOf('.');
	const hasExt = dot > 0 && dot < name.length - 1;
	const stem = hasExt ? name.slice(0, dot) : name;
	const ext = hasExt ? name.slice(dot) : '';
	for (let n = 2; ; n++) {
		const candidate = `${stem} (${n})${ext}`;
		if (!taken.has(candidate.toLowerCase())) return candidate;
	}
}
