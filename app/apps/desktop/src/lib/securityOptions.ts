import { describeMessage } from '$lib/errors';
import { i18n } from '$lib/i18n/index.svelte';
import type { AppSecurity, OptionStatus, VaultPolicy } from '$lib/api/security';

export type PolicyToggle = {
	[K in keyof VaultPolicy]: VaultPolicy[K] extends boolean ? K : never;
}[keyof VaultPolicy];

export type AppToggle = {
	[K in keyof AppSecurity]: AppSecurity[K] extends boolean ? K : never;
}[keyof AppSecurity];

export interface SecurityOption {
	id: string;
	field: PolicyToggle | null;
	appField?: AppToggle;
	title: string;
	protects: string;
	limit: string;
	cost: string;
	inElevated: boolean;
}

export const SECURITY_OPTIONS: SecurityOption[] = [
	{
		id: 'readOnlyDrive',
		field: 'readOnlyDrive',
		get title() {
			return i18n.t.securityOptions.readOnlyDrive.title;
		},
		get protects() {
			return i18n.t.securityOptions.readOnlyDrive.protects;
		},
		get limit() {
			return i18n.t.securityOptions.readOnlyDrive.limit;
		},
		get cost() {
			return i18n.t.securityOptions.readOnlyDrive.cost;
		},
		inElevated: true,
	},
	{
		id: 'noUnlockShortcuts',
		field: 'noUnlockShortcuts',
		get title() {
			return i18n.t.securityOptions.noUnlockShortcuts.title;
		},
		get protects() {
			return i18n.t.securityOptions.noUnlockShortcuts.protects;
		},
		get limit() {
			return i18n.t.securityOptions.noUnlockShortcuts.limit;
		},
		get cost() {
			return i18n.t.securityOptions.noUnlockShortcuts.cost;
		},
		inElevated: true,
	},
	{
		id: 'lockUnmountsDrive',
		field: 'lockUnmountsDrive',
		get title() {
			return i18n.t.securityOptions.lockUnmountsDrive.title;
		},
		get protects() {
			return i18n.t.securityOptions.lockUnmountsDrive.protects;
		},
		get limit() {
			return i18n.t.securityOptions.lockUnmountsDrive.limit;
		},
		get cost() {
			return i18n.t.securityOptions.lockUnmountsDrive.cost;
		},
		inElevated: true,
	},
	{
		id: 'lockOnSessionEvents',
		field: 'lockOnSessionEvents',
		get title() {
			return i18n.t.securityOptions.lockOnSessionEvents.title;
		},
		get protects() {
			return i18n.t.securityOptions.lockOnSessionEvents.protects;
		},
		get limit() {
			return i18n.t.securityOptions.lockOnSessionEvents.limit;
		},
		get cost() {
			return i18n.t.securityOptions.lockOnSessionEvents.cost;
		},
		inElevated: true,
	},
	{
		id: 'blockRemoteSessions',
		field: 'blockRemoteSessions',
		get title() {
			return i18n.t.securityOptions.blockRemoteSessions.title;
		},
		get protects() {
			return i18n.t.securityOptions.blockRemoteSessions.protects;
		},
		get limit() {
			return i18n.t.securityOptions.blockRemoteSessions.limit;
		},
		get cost() {
			return i18n.t.securityOptions.blockRemoteSessions.cost;
		},
		inElevated: true,
	},
	{
		id: 'programAccess',
		field: null,
		get title() {
			return i18n.t.securityOptions.programAccess.title;
		},
		get protects() {
			return i18n.t.securityOptions.programAccess.protects;
		},
		get limit() {
			return i18n.t.securityOptions.programAccess.limit;
		},
		get cost() {
			return i18n.t.securityOptions.programAccess.cost;
		},
		inElevated: false,
	},
	{
		id: 'decoys',
		field: 'decoys',
		get title() {
			return i18n.t.securityOptions.decoys.title;
		},
		get protects() {
			return i18n.t.securityOptions.decoys.protects;
		},
		get limit() {
			return i18n.t.securityOptions.decoys.limit;
		},
		get cost() {
			return i18n.t.securityOptions.decoys.cost;
		},
		inElevated: true,
	},
	{
		id: 'encryptLocalState',
		field: 'encryptLocalState',
		get title() {
			return i18n.t.securityOptions.encryptLocalState.title;
		},
		get protects() {
			return i18n.t.securityOptions.encryptLocalState.protects;
		},
		get limit() {
			return i18n.t.securityOptions.encryptLocalState.limit;
		},
		get cost() {
			return i18n.t.securityOptions.encryptLocalState.cost;
		},
		inElevated: false,
	},
	{
		id: 'disableActivityLog',
		field: 'disableActivityLog',
		get title() {
			return i18n.t.securityOptions.disableActivityLog.title;
		},
		get protects() {
			return i18n.t.securityOptions.disableActivityLog.protects;
		},
		get limit() {
			return i18n.t.securityOptions.disableActivityLog.limit;
		},
		get cost() {
			return i18n.t.securityOptions.disableActivityLog.cost;
		},
		inElevated: false,
	},
	{
		id: 'hideFromCapture',
		field: null,
		appField: 'hideFromCapture',
		get title() {
			return i18n.t.securityOptions.hideFromCapture.title;
		},
		get protects() {
			return i18n.t.securityOptions.hideFromCapture.protects;
		},
		get limit() {
			return i18n.t.securityOptions.hideFromCapture.limit;
		},
		get cost() {
			return i18n.t.securityOptions.hideFromCapture.cost;
		},
		inElevated: false,
	},
	{
		id: 'lockMemory',
		field: null,
		appField: 'lockMemory',
		get title() {
			return i18n.t.securityOptions.lockMemory.title;
		},
		get protects() {
			return i18n.t.securityOptions.lockMemory.protects;
		},
		get limit() {
			return i18n.t.securityOptions.lockMemory.limit;
		},
		get cost() {
			return i18n.t.securityOptions.lockMemory.cost;
		},
		inElevated: false,
	},
	{
		id: 'panic',
		field: null,
		get title() {
			return i18n.t.securityOptions.panic.title;
		},
		get protects() {
			return i18n.t.securityOptions.panic.protects;
		},
		get limit() {
			return i18n.t.securityOptions.panic.limit;
		},
		get cost() {
			return i18n.t.securityOptions.panic.cost;
		},
		inElevated: false,
	},
	{
		id: 'failedUnlockNotice',
		field: null,
		appField: 'failedUnlockNotice',
		get title() {
			return i18n.t.securityOptions.failedUnlockNotice.title;
		},
		get protects() {
			return i18n.t.securityOptions.failedUnlockNotice.protects;
		},
		get limit() {
			return i18n.t.securityOptions.failedUnlockNotice.limit;
		},
		get cost() {
			return i18n.t.securityOptions.failedUnlockNotice.cost;
		},
		inElevated: false,
	},
	{
		id: 'clearRecentsOnExit',
		field: null,
		appField: 'clearRecentsOnExit',
		get title() {
			return i18n.t.securityOptions.clearRecentsOnExit.title;
		},
		get protects() {
			return i18n.t.securityOptions.clearRecentsOnExit.protects;
		},
		get limit() {
			return i18n.t.securityOptions.clearRecentsOnExit.limit;
		},
		get cost() {
			return i18n.t.securityOptions.clearRecentsOnExit.cost;
		},
		inElevated: false,
	},
];

export function isAppOption(option: SecurityOption): boolean {
	return option.appField !== undefined || option.id === 'panic';
}

export type OptionState =
	| { kind: 'available'; caveat: string | null }
	| { kind: 'planned' }
	| { kind: 'unavailable'; reason: string };

export function optionState(statuses: OptionStatus[], id: string): OptionState {
	const status = statuses.find((s) => s.id === id);
	if (!status || status.availability === 'planned') return { kind: 'planned' };
	if (status.availability === 'unavailable') {
		return { kind: 'unavailable', reason: status.reason ? describeMessage(status.reason) : i18n.t.security.unavailableLower };
	}
	return { kind: 'available', caveat: status.caveat ? describeMessage(status.caveat) : null };
}

export function elevatedSummary(statuses: OptionStatus[]): { now: string[]; later: string[] } {
	const now: string[] = [];
	const later: string[] = [];
	for (const option of SECURITY_OPTIONS) {
		if (!option.inElevated) continue;
		const state = optionState(statuses, option.id);
		if (state.kind === 'available') now.push(option.title);
		else if (state.kind === 'planned') later.push(option.title);
	}
	return { now, later };
}
