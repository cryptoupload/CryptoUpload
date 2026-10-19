import codes from './codes.json';
import { en, type Dict } from './locales/en';
import { fill, mergeDict, pickPlural, resolveLanguage, toLanguageInfo, unflatten, type LanguageInfo } from './core';
import type { PluralForms } from './types';
import { uiSettingsGet, uiSettingsSet } from '$lib/api/settings';

export { fill } from './core';
export type { LanguageInfo } from './core';
export type { Dict } from './locales/en';

const loaders = import.meta.glob<Record<string, unknown>>([
	'./locales/*.ts',
	'!./locales/en.ts',
]);

export const languages: LanguageInfo[] = codes
	.filter((c) => c.code === 'en' || `./locales/${c.code}.ts` in loaders)
	.map(toLanguageInfo);

const english = languages.find((l) => l.code === 'en') ?? languages[0];

let dict = $state.raw<Dict>(en);
let info = $state.raw<LanguageInfo>(english);
let preference = $state<string | null>(null);
let saved: Dict = en;
let requested = 0;

export const i18n = {
	get t(): Dict {
		return dict;
	},
	get language(): LanguageInfo {
		return info;
	},
	get preference(): string | null {
		return preference;
	},
};

export function systemLanguage(): LanguageInfo {
	const system = typeof navigator === 'undefined' ? [] : (navigator.languages ?? [navigator.language]);
	const code = resolveLanguage(null, system, languages.map((l) => l.code));
	return languages.find((l) => l.code === code) ?? english;
}

export async function loadDictionary(code: string): Promise<Dict> {
	if (code === 'en') return en;
	const loader = loaders[`./locales/${code}.ts`];
	if (!loader) return en;
	try {
		const mod = await loader();
		return mergeDict(en, mod[code]);
	} catch {
		return en;
	}
}

export const allLanguages: LanguageInfo[] = codes.map(toLanguageInfo);

export async function loadRawDictionary(code: string): Promise<unknown> {
	if (code === 'en') return en;
	const loader = loaders[`./locales/${code}.ts`];
	if (!loader) return null;
	try {
		return (await loader())[code] ?? null;
	} catch {
		return null;
	}
}

export async function showLanguage(code: string): Promise<void> {
	const ticket = ++requested;
	const next = languages.find((l) => l.code === code) ?? english;
	const loaded = await loadDictionary(next.code);
	if (ticket !== requested) return;
	saved = loaded;
	dict = loaded;
	info = next;
	if (typeof document !== 'undefined') {
		document.documentElement.lang = next.tag;
		document.documentElement.dir = next.dir;
	}
}

export async function loadLanguage(): Promise<void> {
	try {
		preference = (await uiSettingsGet()).language ?? null;
	} catch {
		preference = null;
	}
	const available = languages.map((l) => l.code);
	await showLanguage(preference && available.includes(preference) ? preference : systemLanguage().code);
}

export async function setLanguage(code: string | null): Promise<void> {
	preference = code;
	await showLanguage(code ?? systemLanguage().code);
	await uiSettingsSet({ language: code ?? 'system' });
}

export function previewTexts(edits: Iterable<[string, string]>): void {
	dict = mergeDict(saved, unflatten(edits));
}

export function plural(n: number, forms: PluralForms, values: Record<string, string | number> = {}): string {
	return fill(pickPlural(info.tag, n, forms), { n: new Intl.NumberFormat(info.tag).format(n), ...values });
}
