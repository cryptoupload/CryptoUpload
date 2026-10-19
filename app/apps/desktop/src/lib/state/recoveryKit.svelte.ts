import { recoveryKitGenerate } from '$lib/api/recoveryKit';
import { friendlyError } from '$lib/errors';

export const recoveryKitState = $state<{
	html: string | null;
	busy: boolean;
	error: string | null;
}>({
	html: null,
	busy: false,
	error: null,
});

export async function generateRecoveryKit(): Promise<void> {
	if (recoveryKitState.busy) return;
	recoveryKitState.error = null;
	recoveryKitState.busy = true;
	try {
		recoveryKitState.html = await recoveryKitGenerate();
	} catch (e) {
		recoveryKitState.error = friendlyError(e);
	} finally {
		recoveryKitState.busy = false;
	}
}

export function closeRecoveryKitPanel(): void {
	recoveryKitState.html = null;
	recoveryKitState.error = null;
}
