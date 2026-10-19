import fs from 'node:fs';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { availableCodes, codes, isPlural, loadDictionary, root, writeDictionary } from './dictionaries.mjs';

const args = process.argv.slice(2);
const file = args.find((a) => !a.startsWith('--'));
const write = args.includes('--write');
const force = args.includes('--force');
if (!file) { console.log('usage: node tools/apply-translation.mjs <file.json> [--write] [--force]'); process.exit(1); }

const MAX_BYTES = 2 * 1024 * 1024;
const MAX_TEXT = 4000;
const stop = (why) => { console.log(`refused: ${why}`); process.exit(1); };

if (fs.statSync(file).size > MAX_BYTES) stop('the file is too large');
let data;
try { data = JSON.parse(fs.readFileSync(file, 'utf8')); } catch { stop('the file is not JSON'); }
if (!data || typeof data !== 'object' || data.app !== 'cryptoupload-desktop') stop('not a file of the desktop app');
const lang = data.language;
if (typeof lang !== 'string' || !codes.some((c) => c.code === lang) || lang === 'en') stop(`unknown language ${JSON.stringify(lang)}`);
if (!availableCodes().includes(lang)) stop(`${lang} has no dictionary yet: create src/lib/i18n/locales/${lang}.ts first`);
if (!Array.isArray(data.edits)) stop('no list of edits');

const steps = (key) => key.split(/\.|\[(\d+)\]/).filter((p) => p !== undefined && p !== '');
function at(dict, key) {
  let node = dict;
  for (const p of steps(key)) { if (node === null || typeof node !== 'object' || !Object.hasOwn(node, p)) return undefined; node = node[p]; }
  return node;
}
function set(dict, key, value) {
  const parts = steps(key);
  let node = dict;
  for (const p of parts.slice(0, -1)) node = node[p];
  node[parts.at(-1)] = value;
}

const en = await loadDictionary('en');
const dict = await loadDictionary(lang);
const kept = (text, re) => (text.match(re) ?? []).sort().join(' ');
const FORMS = ['zero', 'one', 'two', 'few', 'many', 'other'];

let applied = 0, skipped = 0;
for (const e of data.edits) {
  const note = (why) => { console.log(`skip ${JSON.stringify(e?.key)}: ${why}`); skipped++; };
  if (!e || typeof e.key !== 'string' || typeof e.to !== 'string' || typeof e.from !== 'string') { note('malformed'); continue; }
  if (!/^[\w.[\]]+$/.test(e.key)) { note('bad key'); continue; }
  const parts = steps(e.key);
  const parent = at(en, parts.slice(0, -1).join('.'));
  const pluralForm = isPlural(parent) && FORMS.includes(parts.at(-1));
  const english = pluralForm ? parent.other : at(en, e.key);
  if (typeof english !== 'string') { note('no such text in the app'); continue; }
  const to = e.to.trim();
  if (!to || to.length > MAX_TEXT || /[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(to)) { note('empty, too long or with control characters'); continue; }
  const placeholdersOk = pluralForm
    ? (to.match(/\{\w+\}/g) ?? []).every((p) => english.includes(p))
    : kept(english, /\{\w+\}/g) === kept(to, /\{\w+\}/g);
  if (!placeholdersOk) { note('placeholders differ from English'); continue; }
  if (kept(english, /<\/?\w+>/g) !== kept(to, /<\/?\w+>/g)) { note('markup differs from English'); continue; }
  const now = at(dict, e.key) ?? '';
  if (now === to) { note('already applied'); continue; }
  if (now !== e.from && !force) { note(`the text changed since (now ${JSON.stringify(now)})`); continue; }
  console.log(`${e.key}\n  - ${now}\n  + ${to}`);
  set(dict, e.key, to);
  applied++;
}

console.log(`\n${lang}: ${applied} to apply, ${skipped} skipped. Native speaker: ${data.nativeSpeaker === true ? 'yes' : 'no'}. Licence accepted: ${data.licenceAccepted === true ? 'yes' : 'NO'}.`);
if (data.licenceAccepted !== true) stop('the sender did not accept the licence: nothing can be used');
if (!write) { console.log('Nothing written. Add --write to apply.'); process.exit(0); }
if (applied) {
  writeDictionary(lang, dict);
  execFileSync(process.execPath, [path.join(root, 'tools/check-i18n.mjs')], { stdio: 'inherit' });
}
