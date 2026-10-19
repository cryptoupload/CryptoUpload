import { mountStart, mountStatus, mountStop } from '$lib/api/mount';

export const mount = $state<{
	mounted: boolean;
	mountpoint: string | null;
	supported: boolean;
	busy: boolean;
	readOnly: boolean;
	pendingFiles: number;
	policyReadOnly: boolean;
	confirmWrite: boolean;
}>({
	mounted: false,
	mountpoint: null,
	supported: true,
	busy: false,
	readOnly: false,
	pendingFiles: 0,
	policyReadOnly: false,
	confirmWrite: false,
});

export async function refreshMount(): Promise<void> {
	try {
		const status = await mountStatus();
		mount.mounted = status.mounted;
		mount.mountpoint = status.mountpoint;
		mount.supported = status.supported;
		mount.pendingFiles = status.pendingFiles;
		mount.policyReadOnly = status.policyReadOnly;
		if (!status.policyReadOnly) mount.confirmWrite = false;
		if (status.mounted) mount.readOnly = status.readOnly;
	} catch {
	}
}

export async function toggleMount(): Promise<void> {
	mount.busy = true;
	try {
		if (mount.mounted) {
			await mountStop();
			mount.mounted = false;
			mount.mountpoint = null;
			mount.pendingFiles = 0;
		} else {
			const confirmWrite = mount.policyReadOnly && mount.confirmWrite;
			const status = await mountStart(mount.readOnly && !confirmWrite, confirmWrite);
			mount.mounted = status.mounted;
			mount.mountpoint = status.mountpoint;
			mount.supported = status.supported;
			mount.readOnly = status.readOnly;
			mount.policyReadOnly = status.policyReadOnly;
			mount.confirmWrite = false;
		}
	} finally {
		mount.busy = false;
	}
}

export function resetMount(): void {
	mount.mounted = false;
	mount.mountpoint = null;
	mount.pendingFiles = 0;
	mount.policyReadOnly = false;
	mount.confirmWrite = false;
}
