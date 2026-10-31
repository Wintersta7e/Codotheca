#!/usr/bin/env node
/**
 * Post-pack guard. Run after `npm run package:dir` (or a full target build).
 *
 * The packager fails loudly on a bad config and silently on a bad result. Each silent failure
 * has one assertion here: missing filtered resources, a non-executable Linux core, an asar
 * whose packed manifest does not point at the built main process, or one without the licence
 * and the third-party notices.
 */
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { packedLicenceFailures, readAsarFile } from './lib/asar.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const configPath = join(root, 'electron-builder.yml');
const failures = [];
const fail = (message) => failures.push(message);
let configAssertions = 0;

let configText = '';
try {
  configText = readFileSync(configPath, 'utf8');
} catch (error) {
  fail(`cannot read electron-builder.yml: ${String(error)}`);
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&');
}

function readBlock(source, key, label) {
  const pattern = new RegExp(
    `^${escapeRegExp(key)}:[^\\S\\r\\n]*(?:#[^\\r\\n]*)?\\r?\\n` +
      '([\\s\\S]*?)(?=^(?![ \\t#])[^:\\r\\n]+:[^\\r\\n]*$|(?![\\s\\S]))',
    'mu',
  );
  const match = pattern.exec(source);
  if (match === null) {
    fail(`${label} block was not found in electron-builder.yml`);
    return '';
  }
  return match[1].replace(/^ {2}/gmu, '');
}

function readScalar(source, key, label) {
  const pattern = new RegExp(`^${escapeRegExp(key)}:[^\\S\\r\\n]*([^#\\r\\n]*)`, 'mu');
  const match = pattern.exec(source);
  if (match === null || match[1].trim() === '') {
    fail(`${label} was not found in electron-builder.yml`);
    return undefined;
  }
  const raw = match[1].trim();
  if (
    raw.length >= 2 &&
    ((raw.startsWith("'") && raw.endsWith("'")) || (raw.startsWith('"') && raw.endsWith('"')))
  ) {
    return raw.slice(1, -1);
  }
  return raw;
}

function assertFalse(block, key, label, message) {
  configAssertions += 1;
  const value = readScalar(block, key, label);
  if (value !== undefined && value !== 'false') fail(message);
}

function targetNames(section, sectionName) {
  const targetBlock = readBlock(section, 'target', `${sectionName}.target`);
  const matches = [...targetBlock.matchAll(/^\s*-\s*\{\s*target:\s*([A-Za-z][A-Za-z0-9-]*)\b/gmu)];
  if (matches.length === 0) {
    fail(`${sectionName}.target matched no target entries`);
    return [];
  }
  return matches.map((match) => match[1]);
}

const nsis = readBlock(configText, 'nsis', 'nsis');
assertFalse(
  nsis,
  'perMachine',
  'nsis.perMachine',
  'nsis.perMachine must be false — §14 is per-user',
);
assertFalse(nsis, 'oneClick', 'nsis.oneClick', 'nsis.oneClick must be false');
assertFalse(
  nsis,
  'deleteAppDataOnUninstall',
  'nsis.deleteAppDataOnUninstall',
  'nsis.deleteAppDataOnUninstall must be false — §17 permits no destructive operation',
);

const win = readBlock(configText, 'win', 'win');
// `sign` moved under `signtoolOptions` in electron-builder 26. Reading the nested block keeps
// the assertion pointed at the key the packager actually reads.
const signtool = readBlock(win, 'signtoolOptions', 'win.signtoolOptions');
for (const [label, source, key] of [
  ['afterPack', configText, 'afterPack'],
  ['win.signtoolOptions.sign', signtool, 'sign'],
]) {
  configAssertions += 1;
  const hook = readScalar(source, key, label);
  if (hook !== undefined && !existsSync(join(root, hook))) {
    fail(`${label} names no existing file: ${hook}`);
  }
}

const linux = readBlock(configText, 'linux', 'linux');
const linuxTargets = targetNames(linux, 'linux');
for (const required of ['AppImage', 'deb', 'rpm']) {
  configAssertions += 1;
  if (!linuxTargets.includes(required)) fail(`linux target missing: ${required}`);
}

const windowsTargets = targetNames(win, 'win');
for (const required of ['nsis', 'portable']) {
  configAssertions += 1;
  if (!windowsTargets.includes(required)) fail(`win target missing: ${required}`);
}

configAssertions += 1;
const publish = readScalar(configText, 'publish', 'publish');
if (publish !== undefined && publish !== 'null') {
  fail('publish must be null — builds must not publish or carry update metadata');
}

/** The root package.json of an `app.asar`, parsed. */
function readPackedManifest(archivePath) {
  const contents = readAsarFile(archivePath, 'package.json');
  try {
    return JSON.parse(contents.toString('utf8'));
  } catch (error) {
    throw new Error(`packed package.json is invalid: ${String(error)}`);
  }
}

/**
 * The newest mtime across everything the core is compiled from: its sources, its migrations, and
 * the schema both sides are generated from — a migration is what makes a stale core actively
 * dangerous rather than merely behind.
 *
 * A vanished file is skipped rather than counted, which is the discipline
 * `scripts/lib/read-scanned.mjs` states for every gate that walks a tree.
 */
function newestCoreSourceMs() {
  let newest = 0;
  const touch = (path) => {
    try {
      newest = Math.max(newest, statSync(path).mtimeMs);
    } catch (error) {
      if (!error || error.code !== 'ENOENT') throw error;
    }
  };
  const visit = (path) => {
    let entries;
    try {
      entries = readdirSync(path, { withFileTypes: true });
    } catch (error) {
      if (error && error.code === 'ENOENT') return;
      throw error;
    }
    for (const entry of entries) {
      const child = join(path, entry.name);
      if (entry.isDirectory()) visit(child);
      else touch(child);
    }
  };
  visit(join(root, 'core/src'));
  visit(join(root, 'core/migrations'));
  touch(join(root, 'protocol/schema/protocol.json'));
  return newest;
}

const packs = [
  { dir: join(root, 'dist/win-unpacked'), core: 'codotheca-core.exe', windows: true },
  { dir: join(root, 'dist/linux-unpacked'), core: 'codotheca-core', windows: false },
].filter((pack) => existsSync(pack.dir));
if (packs.length === 0) {
  fail('no unpacked output found — run `npm run package:dir` first');
}

let packedFilesChecked = 0;
for (const pack of packs) {
  const resources = join(pack.dir, 'resources');
  const core = join(resources, 'core', pack.core);
  if (!existsSync(core)) {
    fail(`the core binary was not staged into ${pack.dir}: ${core}`);
  } else {
    packedFilesChecked += 1;
    if (!pack.windows && (statSync(core).mode & 0o111) === 0) {
      fail(`the staged core binary is not executable: ${core}`);
    }
    /**
     * **A core older than the tree it ships with is worse than a missing one.** A missing one
     * fails packaging; a stale one packages clean, launches, and the core exits `StdinEof` within
     * milliseconds because the shell beside it speaks a protocol the core was built before — the
     * window is simply black, with nothing on screen naming the cause.
     *
     * Measured: a Windows core built at 11:16 shipped beside a shell carrying a merge that landed
     * at 15:03 with a new migration and four new commands. `npm run package:win` never rebuilds
     * the core — it only copies whatever is staged — so nothing else in the pipeline notices.
     */
    const built = statSync(core).mtimeMs;
    const newest = newestCoreSourceMs();
    if (newest > built) {
      fail(
        `the staged core predates its own sources: ${core} was built ` +
          `${new Date(built).toISOString()}, newest source ${new Date(newest).toISOString()}. ` +
          'Rebuild the core for this target and repackage — packaging never rebuilds it.',
      );
    } else {
      packedFilesChecked += 1;
    }
  }

  // "Staged for THIS target" is two assertions, not one. A platform's extraResources
  // concatenates with the top-level list rather than replacing it, and a single entry naming
  // both core file names put the other platform's core in the pack — a binary the installing
  // machine cannot run, shipped in silence.
  const foreign = join(resources, 'core', pack.windows ? 'codotheca-core' : 'codotheca-core.exe');
  if (existsSync(foreign)) {
    fail(`${pack.dir} carries the other platform's core: ${foreign}`);
  } else {
    packedFilesChecked += 1;
  }

  if (pack.windows) {
    for (const arch of ['x64', 'arm64']) {
      const worker = join(resources, 'worker', `linux-${arch}`, 'codotheca-worker');
      if (!existsSync(worker)) {
        fail(`the WSL worker was not staged into ${pack.dir}: ${worker}`);
      } else {
        packedFilesChecked += 1;
      }
    }
  }

  const archive = join(resources, 'app.asar');
  if (!existsSync(archive)) {
    fail(`no app.asar in ${resources}`);
    continue;
  }
  packedFilesChecked += 1;

  let manifest;
  try {
    manifest = readPackedManifest(archive);
  } catch (error) {
    fail(`cannot read packed package.json from ${archive}: ${String(error)}`);
    continue;
  }
  if (manifest?.main !== 'app/out/main/index.js') {
    fail(`packed main is "${String(manifest?.main)}"`);
  }

  // The licence and the third-party notices travel inside the archive (`files:`), so a glob
  // that stopped matching ships an artifact without them and the packager says nothing.
  const licenceFailures = packedLicenceFailures(archive);
  for (const message of licenceFailures) fail(message);
  if (licenceFailures.length === 0) packedFilesChecked += 2;
}

if (failures.length > 0) {
  for (const message of failures) console.error(`check-packaging: ${message}`);
  process.exit(1);
}
console.error(
  `check-packaging: ok (${String(configAssertions)} config assertions, ` +
    `${String(packs.length)} unpacked pack(s), ${String(packedFilesChecked)} packed files)`,
);
