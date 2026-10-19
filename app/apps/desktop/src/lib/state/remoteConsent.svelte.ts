export const remoteConsent = $state<{
	programs: string | null;
	allow: boolean;
}>({ programs: null, allow: false });

const CODE = 'REMOTE_SESSION:';

export function noteRemoteRefusal(e: unknown): boolean {
	const text = String(e);
	if (!text.startsWith(CODE)) return false;
	remoteConsent.programs = text.slice(CODE.length);
	return true;
}

export function resetRemoteConsent(): void {
	remoteConsent.programs = null;
	remoteConsent.allow = false;
}
