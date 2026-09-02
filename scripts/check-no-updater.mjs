#!/usr/bin/env node
/**
 * There is no auto-update, so there is no updater.
 *
 * Distribution is a releases page the user downloads from by hand: there is no store, no feed
 * and no update server, so a self-updating artifact would be updating from nothing. That makes
 * "an installed system package never rewrites itself" true by construction rather than by a
 * runtime check — but only for as long as the dependency stays absent, and `electron-updater`
 * is a two-word edit away from being back. This is the zero-call-sites shape: it asserts the
 * name appears in no manifest and in no built bundle, and it prints what it scanned so a run
 * that found nothing to look at fails instead of passing vacuously.
 *
 * Scanning the built bundle and not only the manifests matters because the two fail
 * differently: a manifest entry ships a runtime dependency the packager collects, while an
 * import the bundler inlined ships the same code with no manifest entry at all.
 */
import { existsSync, readdirSync, statSync } from 'node:fs';
import { dirname, extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readScannedFile } from './lib/read-scanned.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const FORBIDDEN = 'electron-updater';
const failures = [];
const fail = (message) => failures.push(message);

// Every gate under scripts/ walks a tree while vitest runs in parallel, and a walked path can
// be gone by the time it is read. Skipped files are not counted, or the "scanned nothing"
// guard below would stop meaning what it says.
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

// 1. No manifest declares it, in any dependency block, in any workspace.
const MANIFESTS = ['package.json', 'app/package.json', 'protocol/package.json'];
const DEPENDENCY_BLOCKS = [
  'dependencies',
  'devDependencies',
  'optionalDependencies',
  'peerDependencies',
];
let manifestsScanned = 0;
for (const manifest of MANIFESTS) {
  const text = readScannedFile(join(root, manifest));
  if (text === null) continue;
  manifestsScanned += 1;
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    fail(`${manifest} is not valid JSON: ${String(error)}`);
    continue;
  }
  for (const block of DEPENDENCY_BLOCKS) {
    const declared = parsed[block];
    if (declared !== undefined && Object.hasOwn(declared, FORBIDDEN)) {
      fail(`${manifest} declares ${FORBIDDEN} in ${block}; there is no update path to use it`);
    }
  }
}

// 2. No built bundle carries it — neither as a runtime require the packager must collect nor
//    as code the bundler inlined.
const bundles = walk(join(root, 'app/out')).filter((f) => {
  const ext = extname(f);
  return ext === '.js' || ext === '.cjs' || ext === '.mjs';
});
let bundlesScanned = 0;
for (const file of bundles) {
  const text = readScannedFile(file);
  if (text === null) continue;
  bundlesScanned += 1;
  if (text.includes(FORBIDDEN)) {
    fail(`${relative(root, file)} references ${FORBIDDEN}`);
  }
}

// 3. A gate whose passing run scanned nothing is a failing gate.
if (manifestsScanned === 0) fail('scanned no manifests — this gate proved nothing');
if (bundlesScanned === 0) {
  fail('scanned no bundle files under app/out — run `npm run build:app` first');
}

if (failures.length > 0) {
  for (const message of failures) console.error(`check-no-updater: ${message}`);
  process.exit(1);
}
console.error(
  `check-no-updater: ok (${String(manifestsScanned)} manifests, ` +
    `${String(bundlesScanned)} bundle files, no ${FORBIDDEN})`,
);
