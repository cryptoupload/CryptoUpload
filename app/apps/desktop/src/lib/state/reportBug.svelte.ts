export const reportBug = $state<{ open: boolean }>({ open: false });

export function openReportBug(): void {
	reportBug.open = true;
}

export function closeReportBug(): void {
	reportBug.open = false;
}
