import type { PluralForms } from './types';

export interface LanguageInfo {
	code: string;
	tag: string;
	name: string;
	dir: 'ltr' | 'rtl';
	draft: boolean;
}

const REVIEWED = new Set(['en']);

export function toLanguageInfo(c: { code: string; tag: string; name: string; dir?: string }): LanguageInfo {
	return { code: c.code, tag: c.tag, name: c.name, dir: c.dir === 'rtl' ? 'rtl' : 'ltr', draft: !REVIEWED.has(c.code) };
}

export function resolveLanguage(preference: string | null, system: readonly string[], available: readonly string[]): string {
	if (preference && available.includes(preference)) return preference;
	for (const raw of system) {
		const tag = raw.toLowerCase();
		if (available.includes(tag)) return tag;
		const base = tag.split(/[-_]/)[0];
		if (available.includes(base)) return base;
	}
	return 'en';
}

export function fill(template: string, values: Record<string, string | number>): string {
	return template.replace(/\{(\w+)\}/g, (whole, key: string) => (key in values ? String(values[key]) : whole));
}

const rules = new Map<string, Intl.PluralRules>();

export function pickPlural(tag: string, n: number, forms: PluralForms): string {
	let rule = rules.get(tag);
	if (!rule) {
		rule = new Intl.PluralRules(tag);
		rules.set(tag, rule);
	}
	return forms[rule.select(n)] ?? forms.other;
}

function isPlain(v: unknown): v is Record<string, unknown> {
	return typeof v === 'object' && v !== null && !Array.isArray(v);
}

export function mergeDict<T>(base: T, over: unknown): T {
	if (typeof base === 'string') return (typeof over === 'string' && over.trim() ? over : base) as T;
	if (Array.isArray(base)) return (Array.isArray(over) ? base.map((b, i) => mergeDict(b, over[i])) : base) as T;
	if (isPlain(base)) {
		if (!isPlain(over)) return base;
		if (typeof base.other === 'string') {
			const forms: Record<string, unknown> = { ...base };
			for (const [form, text] of Object.entries(over)) if (typeof text === 'string' && text.trim()) forms[form] = text;
			return forms as T;
		}
		const out: Record<string, unknown> = {};
		for (const key of Object.keys(base)) out[key] = mergeDict(base[key], over[key]);
		return out as T;
	}
	return base;
}

export function flatten(dict: unknown, prefix = '', out = new Map<string, string>()): Map<string, string> {
	if (typeof dict === 'string') out.set(prefix, dict);
	else if (Array.isArray(dict)) dict.forEach((v, i) => flatten(v, `${prefix}[${i}]`, out));
	else if (isPlain(dict)) for (const [k, v] of Object.entries(dict)) flatten(v, prefix ? `${prefix}.${k}` : k, out);
	return out;
}

export function unflatten(entries: Iterable<[string, string]>): Record<string, unknown> {
	const root: Record<string, unknown> = {};
	for (const [key, text] of entries) {
		const parts = key.split(/\.|\[(\d+)\]/).filter((p) => p !== undefined && p !== '');
		let node: Record<string, unknown> | unknown[] = root;
		parts.forEach((part, i) => {
			const last = i === parts.length - 1;
			const slot = node as Record<string, unknown>;
			if (last) slot[part] = text;
			else {
				if (slot[part] === undefined) slot[part] = /^\d+$/.test(parts[i + 1]) ? [] : {};
				node = slot[part] as Record<string, unknown>;
			}
		});
	}
	return root;
}
