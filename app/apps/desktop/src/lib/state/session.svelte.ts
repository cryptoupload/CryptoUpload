import type { RemoveMemberResult, VaultInfo } from '$lib/api/vault';
import type { FailedUnlock, ForceLockReport } from '$lib/api/security';

export type Screen =
	| 'loading'
	| 'wizard-intro'
	| 'wizard-choice'
	| 'wizard-setup'
	| 'wizard-shortcuts'
	| 'unlock'
	| 'recovery-reveal'
	| 'unlocked'
	| 'remotes'
	| 'deposit'
	| 'open-share';
export type Section = 'vault' | 'browse' | 'jobs' | 'usage' | 'remotes' | 'settings';

export const session = $state<{
	screen: Screen;
	screenBeforeRemotes: Screen;
	screenBeforeDeposit: Screen;
	screenBeforeOpenShare: Screen;
	afterRecoveryScreen: Screen;
	section: Section;
	vaultRoot: string;
	vaultInfo: VaultInfo | null;
	pendingRemoveMemberResult: RemoveMemberResult | null;
	recoveryPhraseConfirmed: boolean;
	autoLockedNotice: boolean;
	forcedLock: ForceLockReport | null;
	failedUnlocks: FailedUnlock[];
	jobRunning: boolean;
	browseDir: string | null;
	opening: boolean;
}>({
	screen: 'loading',
	screenBeforeRemotes: 'unlock',
	screenBeforeDeposit: 'unlock',
	screenBeforeOpenShare: 'unlock',
	afterRecoveryScreen: 'unlocked',
	section: 'vault',
	vaultRoot: '',
	vaultInfo: null,
	pendingRemoveMemberResult: null,
	recoveryPhraseConfirmed: false,
	autoLockedNotice: false,
	forcedLock: null,
	failedUnlocks: [],
	jobRunning: false,
	browseDir: null,
	opening: false,
});
