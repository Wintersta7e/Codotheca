#!/usr/bin/env node
/**
 * Post-build guard. Run after `npm run build:app`.
 *
 * Asserts that the bundle exists where the packaging config expects it, that the renderer's
 * stylesheets and document reach for nothing off the machine, and that the three type
 * families were emitted as local assets.
 */
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const rendererDir = join(root, 'app/out/renderer');
const failures = [];

function fail(message) {
  failures.push(message);
}

function walk(dir) {
  if (!existsSync(dir)) return [];
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...walk(full));
    else out.push(full);
  }
  return out;
}

// 1. The bundle is where app/package.json and electron-builder.yml say it is.
for (const required of [
  'app/out/main/index.js',
  'app/out/preload/index.js',
  'app/out/renderer/index.html',
]) {
  if (!existsSync(join(root, required))) fail(`missing build output: ${required}`);
}

const files = walk(rendererDir);

// 2. No remote origin in the document or in any stylesheet.
//    Only .html and .css are scanned. React's production build embeds
//    https://react.dev/errors/… in its minified invariant messages, so scanning .js would
//    report a string that is never fetched and can never be removed.
const REMOTE = /\b(?:https?:)?\/\/(?!\/)[a-z0-9.-]+/giu;
for (const file of files) {
  const ext = extname(file);
  if (ext !== '.html' && ext !== '.css') continue;
  const hits = readFileSync(file, 'utf8').match(REMOTE);
  if (hits !== null) {
    fail(`remote origin in ${relative(root, file)}: ${[...new Set(hits)].join(', ')}`);
  }
}

// 3. The policy actually shipped in the document.
const indexHtml = existsSync(join(rendererDir, 'index.html'))
  ? readFileSync(join(rendererDir, 'index.html'), 'utf8')
  : '';
for (const clause of ["default-src 'none'", "font-src 'self'"]) {
  if (!indexHtml.includes(clause) && !indexHtml.includes(clause.replaceAll("'", '&#39;'))) {
    fail(`index.html does not carry the policy clause: ${clause}`);
  }
}

/**
 * 3a. The screens are actually in the bundle.
 *
 * Every screen in this product was built, tested in jsdom, and mounted by nothing for two
 * sessions — and the bundler dutifully tree-shook all of it out, which nothing noticed because
 * the suite was green and the build succeeded. This is the check that would have said so.
 *
 * Matched as `function <Name>(`, which is what this build emits: it does not mangle names. If
 * minification is ever turned on this gate goes red rather than quietly passing, and whoever
 * turns it on owns changing the detector.
 */
export const MOUNTED_MODULES = [
  'FirstRunGate',
  'TurnScreen',
  'Shelf',
  'GridSection',
  'SettingsDrawer',
  'ScanSummary',
  'FailureWindow',
  'QuickSwitch',
  'ProjectPageView',
];

const scripts = files.filter((f) => extname(f) === '.js');
const bundleText = scripts.map((f) => readFileSync(f, 'utf8')).join('\n');
// A gate whose passing run scans zero files is a failing gate.
if (scripts.length === 0) {
  fail('no renderer script was emitted, so the mounted-module check scanned nothing');
} else {
  const missing = MOUNTED_MODULES.filter((name) => !bundleText.includes(`function ${name}(`));
  if (missing.length > 0) {
    fail(
      `${String(missing.length)} of ${String(MOUNTED_MODULES.length)} screens are absent from ` +
        `the bundle (tree-shaken because nothing mounts them): ${missing.join(', ')}`,
    );
  }
}

// 4. The three families were emitted as local assets.
const woff2 = files.filter((f) => extname(f) === '.woff2');
for (const family of ['rajdhani', 'barlow', 'jetbrains-mono']) {
  if (!woff2.some((f) => f.toLowerCase().includes(family))) {
    fail(`no self-hosted woff2 emitted for ${family}`);
  }
}

// 5. Every packaging glob matches something. electron-builder silently packages an empty
//    app if `files:` points at a directory that the bundler stopped writing to — which is
//    exactly what a bundler migration does to a config nobody re-read.
const yamlText = readFileSync(join(root, 'electron-builder.yml'), 'utf8');
const filesBlock = /^files:\n((?:\s*(?:#.*|-\s.+)\n)+)/mu.exec(yamlText);
if (filesBlock === null) {
  fail('electron-builder.yml declares no files: list');
} else {
  for (const line of filesBlock[1].split('\n')) {
    if (line.trim().startsWith('#')) continue;
    const glob = line.replace(/^\s*-\s*/u, '').trim();
    if (glob === '') continue;
    // The only glob shapes this config uses are `dir/**` and a bare path.
    const target = glob.endsWith('/**') ? glob.slice(0, -3) : glob;
    if (!existsSync(join(root, target))) {
      fail(`packaging glob matches nothing: ${glob}`);
    }
  }
}

// 6. The entry electron-builder packages resolves.
const rootPackage = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
if (typeof rootPackage.main !== 'string') {
  fail('the root package.json declares no main, so electron-builder has no entry point');
} else if (!existsSync(join(root, rootPackage.main))) {
  fail(`the root package.json main does not exist: ${rootPackage.main}`);
}

if (failures.length > 0) {
  for (const message of failures) console.error(`check-bundle: ${message}`);
  process.exit(1);
}
console.error(
  `check-bundle: ok (${String(files.length)} renderer files, ${String(woff2.length)} woff2, ` +
    `${String(MOUNTED_MODULES.length)} screens present in ${String(scripts.length)} script(s))`,
);
