import { i18n } from '$lib/i18n/index.svelte';

export function createMessages(): readonly string[] {
	return [i18n.t.busy.generatingKey, i18n.t.busy.creatingVault];
}

export function createBusyMessages(intervalMs = 550) {
	let message = $state('');
	let timer: ReturnType<typeof setInterval> | undefined;

	function stop() {
		clearInterval(timer);
		timer = undefined;
	}

	return {
		get message() {
			return message;
		},
		start(messages: readonly string[]) {
			stop();
			let i = 0;
			message = messages[0] ?? '';
			timer = setInterval(() => {
				i = Math.min(i + 1, messages.length - 1);
				message = messages[i];
			}, intervalMs);
		},
		stop,
	};
}
