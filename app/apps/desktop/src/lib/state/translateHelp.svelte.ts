export const translateHelp = $state<{ open: boolean }>({ open: false });

export function openTranslateHelp(): void {
	translateHelp.open = true;
}

export function closeTranslateHelp(): void {
	translateHelp.open = false;
}
