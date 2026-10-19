import fs from 'node:fs';
import path from 'node:path';
import { parse } from 'svelte/compiler';
import { root } from './dictionaries.mjs';

const src = path.join(root, 'src');
const LOOSE = process.argv.includes('--loose');
const SKIP = [
  /\.d\.ts$/,
  /[\\/]i18n[\\/]/,
];
const TEXT_ATTRS = new Set(['placeholder', 'aria-label', 'title', 'alt', 'label', 'aria-description', 'aria-roledescription', 'aria-valuetext']);
const TEXT_PROPS = /^(title|message|label|confirmLabel|cancelLabel|hint|description|heading|text|placeholder|emptyText|detail|note)$/;
const SILENT_CALLS = /^(invoke|listen|emit|querySelector|querySelectorAll|getElementById|addEventListener|removeEventListener|setAttribute|getAttribute|removeAttribute|hasAttribute|matchMedia|closest|matches|startsWith|endsWith|includes|indexOf|split|join|replace|replaceAll|padStart|padEnd|getItem|setItem|removeItem|toLocaleString|toLocaleDateString|toLocaleTimeString|import|require|createElement|add|remove|toggle|contains|has|get|set|delete|log|warn|error|debug|test|match|localeCompare|open|Error)$/;

function files(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) return files(p);
    return /\.(svelte|ts)$/.test(e.name) && !SKIP.some((re) => re.test(p)) ? [p] : [];
  });
}

function looksLikeText(s, template = false) {
  const t = s.trim();
  if (!/\p{L}{2,}/u.test(t)) return false;
  if (KEYWORDS.has(t) || /^[a-z0-9+]+:\/\/\S*$/.test(t)) return false;
  if (template) return true;
  if (/^(https?:|mailto:|\$lib|@)/.test(t)) return false;
  if (/\p{L}\s+\p{L}/u.test(t) || /\p{L}[,.:;!?…]\s/u.test(t)) return true;
  if (/^\p{Lu}\p{Ll}+[:…!?.]*$/u.test(t)) return true;
  if (/\d\s+\p{L}{2,}/u.test(t) || /^\p{Ll}{3,}[,:;]$/u.test(t)) return true;
  if (LOOSE) return !/^[a-z][A-Za-z0-9_]*$/.test(t) && !/^[\w$.:/#@=\-\[\]()*,>+~|%!?&;{}]+$/.test(t);
  return false;
}
const KEYWORDS = new Set(['Enter', 'Escape', 'Tab', 'Backspace', 'Delete', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'PageUp', 'PageDown', 'Shift', 'Control', 'Alt', 'Meta', 'Space', 'Vault', 'CryptoUpload', 'SFTP', 'WebDAV', 'Dropbox', 'OneDrive', 'Google Drive', 'S3', 'remotes.secrets']);

function calleeName(node) {
  const c = node.callee;
  if (!c) return '';
  if (c.type === 'Identifier') return c.name;
  if (c.type === 'MemberExpression' && c.property?.type === 'Identifier') return c.property.name;
  if (c.type === 'Import') return 'import';
  return '';
}

function scan(file) {
  const text = fs.readFileSync(file, 'utf8');
  const lines = text.split('\n');
  const isSvelte = file.endsWith('.svelte');
  const source = isSvelte ? text : `<script lang="ts">${text}</script>`;
  const shift = isSvelte ? 0 : '<script lang="ts">'.length;
  const ast = parse(source, { modern: true, filename: file });
  const out = [];
  const lineOf = (pos) => text.slice(0, Math.max(0, pos - shift)).split('\n').length;
  const report = (pos, value) => {
    const line = lineOf(pos);
    if (/i18n-ignore/.test(lines[line - 1] ?? '') || /i18n-ignore/.test(lines[line - 2] ?? '')) return;
    out.push({ line, value: value.trim().replace(/\s+/g, ' ') });
  };

  function walk(node, ctx) {
    if (!node || typeof node !== 'object') return;
    if (Array.isArray(node)) return node.forEach((n) => walk(n, ctx));
    if (typeof node.type !== 'string') {
      for (const v of Object.values(node)) if (v && typeof v === 'object') walk(v, ctx);
      return;
    }
    switch (node.type) {
      case 'StyleSheet': case 'Comment': case 'ImportDeclaration': case 'ExportAllDeclaration':
      case 'TSTypeAnnotation': case 'TSTypeAliasDeclaration': case 'TSInterfaceDeclaration': case 'TSLiteralType':
      case 'TSTypeParameterInstantiation': case 'TSAsExpression' && 'never':
        return;
      case 'TSAsExpression': case 'TSSatisfiesExpression': case 'TSNonNullExpression':
        return walk(node.expression, ctx);
      case 'RegularElement':
        if (node.name === 'svg' || node.name === 'style') return;
        break;
      case 'Attribute': {
        const name = node.name;
        const readable = ctx.component ? TEXT_PROPS.test(name) || TEXT_ATTRS.has(name) : TEXT_ATTRS.has(name);
        if (!readable) {
          const parts = Array.isArray(node.value) ? node.value : [node.value];
          for (const part of parts) if (part && part.type === 'ExpressionTag' && !/^(class|style|id|data-|type|name|for|href|src|role|slot|use|bind|on)/.test(name)) walk(part, { ...ctx, quiet: true });
          return;
        }
        const parts = Array.isArray(node.value) ? node.value : [node.value];
        for (const part of parts) {
          if (part && part.type === 'Text' && looksLikeText(part.data, true)) report(part.start, part.data);
          else walk(part, ctx);
        }
        return;
      }
      case 'Text':
        if (ctx.template && looksLikeText(node.data, true)) report(node.start, node.data);
        return;
      case 'Component':
        for (const a of node.attributes ?? []) walk(a, { ...ctx, component: true });
        return walk(node.fragment, { ...ctx, component: false });
      case 'CallExpression': case 'NewExpression':
        walk(node.callee, ctx);
        if (SILENT_CALLS.test(calleeName(node))) return;
        return walk(node.arguments, ctx);
      case 'BinaryExpression':
        if (/^[!=]==?$/.test(node.operator)) {
          if (node.left.type !== 'Literal') walk(node.left, ctx);
          if (node.right.type !== 'Literal') walk(node.right, ctx);
          return;
        }
        break;
      case 'SwitchCase':
        return walk(node.consequent, ctx);
      case 'Property':
        if (node.computed) walk(node.key, ctx);
        return walk(node.value, ctx);
      case 'MemberExpression':
        walk(node.object, ctx);
        if (node.computed && node.property.type !== 'Literal') walk(node.property, ctx);
        return;
      case 'Literal':
        if (typeof node.value === 'string' && !ctx.quiet && looksLikeText(node.value)) report(node.start, node.value);
        return;
      case 'TemplateElement':
        if (!ctx.quiet && looksLikeText(node.value?.cooked ?? '')) report(node.start, node.value.cooked);
        return;
    }
    for (const [k, v] of Object.entries(node)) {
      if (k === 'type' || k === 'start' || k === 'end' || k === 'loc' || k === 'leadingComments' || k === 'trailingComments' || !v || typeof v !== 'object') continue;
      walk(v, k === 'fragment' ? { ...ctx, template: true } : ctx);
    }
  }
  walk(ast.instance, { template: false });
  walk(ast.module, { template: false });
  if (isSvelte) walk(ast.fragment, { template: true });
  return out.sort((a, b) => a.line - b.line);
}

const args = process.argv.slice(2).filter((a) => !a.startsWith('--'));
if (args.length) {
  for (const f of args) for (const h of scan(path.resolve(f))) console.log(`${f}:${h.line}: ${h.value}`);
  process.exit(0);
}
let total = 0;
for (const f of files(src)) {
  const n = scan(f).length;
  if (n) { console.log(`${String(n).padStart(5)}  ${path.relative(root, f)}`); total += n; }
}
console.log(total ? `hardcoded texts: ${total}` : 'no hardcoded texts');
process.exit(total ? 1 : 0);
