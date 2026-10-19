import { i18n } from '$lib/i18n/index.svelte';

export type PasswordStrength = {
	score: 0 | 1 | 2 | 3;
	label: string;
};

function characterClasses(password: string): number {
	let classes = 0;
	if (/[a-z]/.test(password)) classes++;
	if (/[A-Z]/.test(password)) classes++;
	if (/[0-9]/.test(password)) classes++;
	if (/[^a-zA-Z0-9]/.test(password)) classes++;
	return classes;
}

export function passwordStrength(password: string): PasswordStrength {
	if (password.length === 0) {
		return { score: 0, label: i18n.t.vaultForm.strengthLabels[0] };
	}
	const classes = characterClasses(password);
	const effectiveLength = password.length * (classes >= 2 ? 1 : 0.5);

	let score: PasswordStrength['score'];
	if (effectiveLength < 8) score = 0;
	else if (effectiveLength < 12) score = 1;
	else if (effectiveLength < 20) score = 2;
	else score = 3;

	return { score, label: i18n.t.vaultForm.strengthLabels[score] };
}
