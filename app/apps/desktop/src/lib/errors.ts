import { fill, i18n, type Dict } from '$lib/i18n/index.svelte';

type Code = keyof Dict['errors']['codes'];
type Hint = keyof Dict['errors']['hints'];

const CODE_HINTS: Partial<Record<Code, Hint>> = {
	POLICY_DRIVE_READ_ONLY: 'policy',
	POLICY_DRIVE_PROGRAMS_UNSUPPORTED: 'policy',
	POLICY_NO_UNLOCK_SHORTCUTS: 'policy',
	DECRYPT_FAILED: 'decrypt',
	PATH_NOT_FOUND: 'notFound',
	OBJECT_NOT_FOUND: 'notFound',
	NOT_A_VAULT: 'notAVault',
	OBJECT_EXISTS: 'exists',
	DESTINATION_EXISTS: 'exists',
	TRANSIENT: 'transient',
	IO: 'io',
	REGISTRY_MISMATCH: 'registryMismatch',
	REGISTRY_SERVED_DIFFERENT: 'registryMismatch',
	REGISTRY_NOT_REGISTERED: 'registryUnknown',
};

const ARG_SEP = '\u001f';
const LIST_SEP = '\u001e';

function parseCode(raw: string): { code: Code; detail: string; args: string[] } | null {
	const m = /^([A-Z][A-Z0-9_]{2,})(?::([\s\S]*))?$/.exec(raw);
	if (!m || !(m[1] in i18n.t.errors.codes)) return null;
	const detail = m[2] ?? '';
	return { code: m[1] as Code, detail, args: detail === '' ? [] : detail.split(ARG_SEP) };
}

function withHint(message: string, hint: Hint | undefined): string {
	return hint ? fill(i18n.t.errors.withHint, { message, hint: i18n.t.errors.hints[hint] }) : message;
}

function describeCoded(raw: string): string | null {
	const coded = parseCode(raw);
	if (!coded) return null;
	const values: Record<string, string> = { detail: coded.detail.replaceAll(ARG_SEP, ', ') };
	coded.args.forEach((arg, i) => {
		values[String(i)] = arg;
	});
	return fill(i18n.t.errors.codes[coded.code], values);
}

export function describeMessage(raw: string): string {
	return raw
		.split(LIST_SEP)
		.map((part) => describeCoded(part) ?? part)
		.join('; ');
}

export function friendlyError(e: unknown): string {
	const raw = String(e);
	const coded = parseCode(raw);
	if (coded) return withHint(describeCoded(raw) ?? raw, CODE_HINTS[coded.code]);
	return describeMessage(raw);
}

const OFFLINE_MARKERS = [
	'connection refused', // i18n-ignore
	'connection reset', // i18n-ignore
	'timed out', // i18n-ignore
	'timeout',
	'dns',
	'failed to lookup', // i18n-ignore
	'network is unreachable', // i18n-ignore
	'no route to host', // i18n-ignore
	'error sending request', // i18n-ignore
];

export function isOfflineError(message: string): boolean {
	if (parseCode(message)?.code === 'TRANSIENT') return true;
	const lower = message.toLowerCase();
	return OFFLINE_MARKERS.some((m) => lower.includes(m));
}
