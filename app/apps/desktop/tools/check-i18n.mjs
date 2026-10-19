import fs from 'node:fs';
import path from 'node:path';
import { availableCodes, codes, isPlural, loadDictionary, localesDir } from './dictionaries.mjs';

const PLURAL_FORMS = ['zero', 'one', 'two', 'few', 'many', 'other'];
const NAMES = /CryptoUpload|WinFsp|FUSE|FIDO2|SLIP-39|WebDAV|SFTP|S3|OneDrive|Dropbox|Google Drive|GPLv3|Argon2id|AES-256-GCM/g;
const KEPT = [
  ['placeholder', /\{\w+\}/g],
  ['link', /https?:\/\/[^\s)'"<]+/g],
  ['email', /[\w.-]+@[\w.-]+\.\w+/g],
  ['name', NAMES],
];

let bad = 0;
const fail = (lang, text) => { console.log(`${lang}: ${text}`); bad++; };
const found = (text, re) => (text.match(re) ?? []).sort().join('|');

function checkText(lang, p, a, b, plural) {
  if (typeof b !== 'string' || !b.trim()) return fail(lang, `empty ${p}`);
  if (b !== b.trim()) fail(lang, `leading or trailing space at ${p}`);
  for (const [label, re] of KEPT) {
    const ok = plural && label === 'placeholder'
      ? (b.match(re) ?? []).every((x) => (a.match(re) ?? []).includes(x))
      : found(a, re) === found(b, re);
    if (!ok) fail(lang, `${label} differs at ${p}: ${JSON.stringify(a.match(re))} vs ${JSON.stringify(b.match(re))}`);
  }
}

function walk(lang, tag, a, b, p) {
  if (typeof a === 'string') return checkText(lang, p, a, b, false);
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return fail(lang, `${p} has ${b?.length} items, English has ${a.length}`);
    return a.forEach((x, i) => walk(lang, tag, x, b[i], `${p}[${i}]`));
  }
  if (isPlural(a)) {
    if (!isPlural(b)) return fail(lang, `${p} must be a plural set with "other"`);
    const all = new Intl.PluralRules(tag).resolvedOptions().pluralCategories;
    const needed = all.includes('few') ? all : all.filter((f) => f !== 'many');
    for (const form of needed) if (typeof b[form] !== 'string') fail(lang, `missing plural form ${p}.${form}`);
    for (const form of Object.keys(b)) {
      if (!PLURAL_FORMS.includes(form)) fail(lang, `unknown plural form ${p}.${form}`);
      else checkText(lang, `${p}.${form}`, a.other, b[form], true);
    }
    return;
  }
  if (!b || typeof b !== 'object') return fail(lang, `missing ${p}`);
  for (const k of Object.keys(a)) { if (!(k in b)) fail(lang, `missing ${p ? p + '.' : ''}${k}`); else walk(lang, tag, a[k], b[k], p ? `${p}.${k}` : k); }
  for (const k of Object.keys(b)) if (!(k in a)) fail(lang, `extra ${p ? p + '.' : ''}${k}`);
}

const known = new Set(codes.map((c) => c.code));
for (const f of fs.readdirSync(localesDir)) {
  const code = path.basename(f, '.ts');
  if (f.endsWith('.ts') && !known.has(code)) fail(code, `${f} is not listed in codes.json`);
}

const langs = availableCodes();
const en = await loadDictionary('en');
for (const code of langs) walk(code, codes.find((c) => c.code === code).tag, en, code === 'en' ? en : await loadDictionary(code), '');

console.log(bad ? `i18n: ${bad} problem(s)` : `i18n OK (${langs.join(', ')})`);
process.exit(bad ? 1 : 0);
