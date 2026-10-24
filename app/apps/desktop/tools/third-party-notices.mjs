import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const app = path.resolve(desktop, '..', '..');
const outArg = process.argv.indexOf('--out');
const out = outArg > 0 ? path.resolve(process.argv[outArg + 1]) : path.join(desktop, 'src-tauri', 'THIRD-PARTY-NOTICES.txt');

const ROOTS = ['svelte', '@sveltejs/kit'];
const lock = JSON.parse(fs.readFileSync(path.join(desktop, 'package-lock.json'), 'utf8')).packages;
const manifest = JSON.parse(fs.readFileSync(path.join(desktop, 'package.json'), 'utf8'));

function entry(name) {
  const e = lock[`node_modules/${name}`];
  if (!e) throw new Error(`${name} is not in package-lock.json`);
  return e;
}

const seen = new Set();
const queue = [...Object.keys(manifest.dependencies ?? {}), ...ROOTS];
while (queue.length) {
  const name = queue.shift();
  if (seen.has(name)) continue;
  seen.add(name);
  queue.push(...Object.keys(entry(name).dependencies ?? {}));
}

const LICENCE_FILE = /^(licen[cs]e|copying|ofl)(\.(md|txt))?$/i;
function licenceText(name) {
  const dir = path.join(desktop, 'node_modules', name);
  const file = fs.readdirSync(dir).find((f) => LICENCE_FILE.test(f));
  if (file) return fs.readFileSync(path.join(dir, file), 'utf8').replace(/\r\n/g, '\n').trim();
  const pkg = JSON.parse(fs.readFileSync(path.join(dir, 'package.json'), 'utf8'));
  const author = typeof pkg.author === 'string' ? pkg.author : pkg.author?.name;
  const repo = typeof pkg.repository === 'string' ? pkg.repository : pkg.repository?.url;
  if (!pkg.license) throw new Error(`${name}: no licence file and no licence in package.json`);
  return [`Licence: ${pkg.license} (no licence file in the package)`, author && `Author: ${author}`, repo && `Source: ${repo}`]
    .filter(Boolean)
    .join('\n');
}

const rule = '='.repeat(80);
const npm = [...seen].sort().map((name) => {
  const pkg = JSON.parse(fs.readFileSync(path.join(desktop, 'node_modules', name, 'package.json'), 'utf8'));
  return `${rule}\n${name} ${entry(name).version} (${pkg.license ?? 'see below'})\n${'-'.repeat(80)}\n${licenceText(name)}\n`;
});

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'cu-notices-'));
const rustFile = path.join(tmp, 'rust.txt');
execFileSync(
  'cargo',
  [
    'about', 'generate', '--locked', '--all-features',
    '-c', path.join(app, 'about.toml'),
    '--manifest-path', path.join(desktop, 'src-tauri', 'Cargo.toml'),
    '-o', rustFile,
    path.join(app, 'about.hbs'),
  ],
  { cwd: app, stdio: ['ignore', 'ignore', 'inherit'] },
);
const rust = fs.readFileSync(rustFile, 'utf8').replace(/\r\n/g, '\n').trim();
fs.rmSync(tmp, { recursive: true, force: true });

const text = [
  'CryptoUpload includes software written by others. Their licences and notices follow.',
  'CryptoUpload itself is licensed under the GNU GPLv3 with an additional permission for app stores',
  '(LICENSE and LICENSE-EXCEPTION.txt in the source code).',
  '',
  `${rule}\nPART 1: Rust crates\n${rule}`,
  rust,
  '',
  `${rule}\nPART 2: packages used by the app window (fonts and interface code)\n${rule}`,
  ...npm,
].join('\n').replace(/[ \t]+$/gm, '') + '\n';

fs.writeFileSync(out, text);
console.log(`wrote ${out} (${seen.size} npm packages)`);
