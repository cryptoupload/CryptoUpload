import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
export const localesDir = path.join(root, 'src/lib/i18n/locales');
export const codes = JSON.parse(fs.readFileSync(path.join(root, 'src/lib/i18n/codes.json'), 'utf8'));

export async function loadDictionary(code) {
  const src = fs.readFileSync(path.join(localesDir, `${code}.ts`), 'utf8')
    .replace(/^import type .*$/gm, '')
    .replace(/^export type .*$/gm, '')
    .replace(/:\s*Dict\s*=/, ' =');
  const mod = await import('data:text/javascript;base64,' + Buffer.from(src).toString('base64'));
  return mod[code];
}

export function availableCodes() {
  return codes.map((c) => c.code).filter((c) => fs.existsSync(path.join(localesDir, `${c}.ts`)));
}

export function isPlural(v) {
  return v && typeof v === 'object' && !Array.isArray(v) && typeof v.other === 'string';
}

const ident = /^[A-Za-z_$][\w$]*$/;
const quote = (s) => "'" + s.replace(/\\/g, '\\\\').replace(/'/g, "\\'").replace(/\n/g, '\\n') + "'";

function literal(v, depth) {
  const pad = '\t'.repeat(depth);
  if (typeof v === 'string') return quote(v);
  if (Array.isArray(v)) return `[\n${v.map((x) => `${pad}\t${literal(x, depth + 1)},\n`).join('')}${pad}]`;
  return `{\n${Object.entries(v).map(([k, x]) => `${pad}\t${ident.test(k) ? k : quote(k)}: ${literal(x, depth + 1)},\n`).join('')}${pad}}`;
}

export function writeDictionary(code, dict) {
  const head = code === 'en'
    ? `import type { Localized } from '../types';\n\nexport const en = `
    : `import type { Dict } from './en';\n\nexport const ${code}: Dict = `;
  const tail = code === 'en' ? ';\n\nexport type Dict = Localized<typeof en>;\n' : ';\n';
  fs.writeFileSync(path.join(localesDir, `${code}.ts`), head + literal(dict, 0) + tail);
}
