export const TRANSLATIONS_EMAIL = 'translations@cryptoupload.org';

export interface Edit {
	key: string;
	from: string;
	to: string;
}

export interface Contribution {
	app: 'cryptoupload-desktop';
	language: string;
	nativeSpeaker: boolean;
	credit: string;
	contact: string;
	licenceAccepted: boolean;
	edits: Edit[];
}

const placeholders = (text: string) => (text.match(/\{\w+\}/g) ?? []).sort().join(' ');
const tags = (text: string) => (text.match(/<\/?\w+>/g) ?? []).sort().join(' ');

export function editProblem(english: string, text: string, plural: boolean): 'placeholders' | 'markup' | null {
	const kept = plural
		? (text.match(/\{\w+\}/g) ?? []).every((p) => english.includes(p))
		: placeholders(english) === placeholders(text);
	if (!kept) return 'placeholders';
	if (tags(english) !== tags(text)) return 'markup';
	return null;
}

export function buildContribution(
	language: string,
	edits: ReadonlyMap<string, string>,
	current: ReadonlyMap<string, string>,
	form: { nativeSpeaker: boolean; credit: string; contact: string; licenceAccepted: boolean },
): Contribution {
	const list: Edit[] = [];
	for (const [key, to] of edits) {
		const from = current.get(key) ?? '';
		if (to.trim() && to.trim() !== from) list.push({ key, from, to: to.trim() });
	}
	return {
		app: 'cryptoupload-desktop',
		language,
		nativeSpeaker: form.nativeSpeaker,
		credit: form.credit.trim().slice(0, 80),
		contact: form.contact.trim().slice(0, 120),
		licenceAccepted: form.licenceAccepted,
		edits: list,
	};
}

export function contributionFileName(language: string): string {
	return `cryptoupload-translation-${language}.json`;
}

const MAILTO_BODY_MAX = 1500;

export function mailtoUrl(c: Contribution): string {
	const head = [
		`Language: ${c.language}`,
		`Native speaker: ${c.nativeSpeaker ? 'yes' : 'no'}`,
		`Credit: ${c.credit || '-'}`,
		`Licence (GNU GPLv3 with the app store exception) accepted: ${c.licenceAccepted ? 'yes' : 'no'}`,
		'',
	];
	const lines = c.edits.flatMap((e) => [e.key, `  was: ${e.from || '-'}`, `  now: ${e.to}`, '']);
	const full = [...head, ...lines].join('\n');
	const body =
		full.length <= MAILTO_BODY_MAX
			? full
			: [...head, `${c.edits.length} changes: please attach the file ${contributionFileName(c.language)} saved from the app.`].join('\n');
	const subject = `CryptoUpload app translation: ${c.language} (${c.edits.length})`;
	return `mailto:${TRANSLATIONS_EMAIL}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
}
