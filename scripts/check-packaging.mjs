#!/usr/bin/env node
/**
 * Post-pack guard. Run after `npm run package:dir` (or a full target build).
 *
 * The packager fails loudly on a bad config and silently on a bad result. Each silent failure
 * has one assertion here: missing filtered resources, a non-executable Linux core, or an asar
 * whose packed manifest does not point at the built main process.
 */
import { closeSync, existsSync, openSync, readFileSync, readSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

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

function readExactly(descriptor, size, position) {
  const output = Buffer.alloc(size);
  let read = 0;
  while (read < size) {
    const count = readSync(descriptor, output, read, size - read, position + read);
    if (count === 0) break;
    read += count;
  }
  if (read !== size) {
    throw new Error(`unexpected end of file at ${String(position + read)}`);
  }
  return output;
}

/**
 * Reads only the root package.json. An asar starts with an 8-byte pickle: uint32 `4`, then the
 * complete inner-pickle size. The inner pickle is uint32 payload size, uint32 JSON byte length,
 * JSON, then four-byte padding. Packed entry offsets are decimal strings relative to the byte
 * immediately after those two pickles.
 */
function readPackedManifest(archivePath) {
  const archiveSize = statSync(archivePath).size;
  if (archiveSize < 16) throw new Error('archive is shorter than its two pickle headers');

  const descriptor = openSync(archivePath, 'r');
  try {
    const prefix = readExactly(descriptor, 8, 0);
    if (prefix.readUInt32LE(0) !== 4) {
      throw new Error('outer pickle payload size is not 4');
    }

    const headerSize = prefix.readUInt32LE(4);
    if (headerSize < 8) throw new Error('inner pickle is shorter than 8 bytes');
    if (headerSize > 64 * 1024 * 1024) throw new Error('inner pickle exceeds 64 MiB');
    if (8 + headerSize > archiveSize) throw new Error('inner pickle extends past the archive');

    const header = readExactly(descriptor, headerSize, 8);
    const payloadSize = header.readUInt32LE(0);
    if (payloadSize !== headerSize - 4) {
      throw new Error('inner pickle payload size does not match its header');
    }
    const jsonSize = header.readUInt32LE(4);
    if (jsonSize > payloadSize - 4) throw new Error('header JSON extends past the inner pickle');
    const paddedPayloadSize = Math.ceil((4 + jsonSize) / 4) * 4;
    if (paddedPayloadSize !== payloadSize) {
      throw new Error('inner pickle padding is inconsistent');
    }

    let tree;
    try {
      tree = JSON.parse(header.subarray(8, 8 + jsonSize).toString('utf8'));
    } catch (error) {
      throw new Error(`header JSON is invalid: ${String(error)}`);
    }
    const entry = tree?.files?.['package.json'];
    if (entry === undefined || entry === null || typeof entry !== 'object') {
      throw new Error('asar header has no root package.json entry');
    }
    if (entry.unpacked === true) throw new Error('root package.json is unexpectedly unpacked');
    if (Object.hasOwn(entry, 'link')) throw new Error('root package.json is unexpectedly a link');
    if (typeof entry.offset !== 'string' || !/^(?:0|[1-9]\d*)$/u.test(entry.offset)) {
      throw new Error('root package.json has an invalid packed offset');
    }
    if (!Number.isSafeInteger(entry.size) || entry.size < 0) {
      throw new Error('root package.json has an invalid size');
    }

    const filePositionBig = 8n + BigInt(headerSize) + BigInt(entry.offset);
    const fileEndBig = filePositionBig + BigInt(entry.size);
    if (fileEndBig > BigInt(archiveSize)) {
      throw new Error('root package.json extends past the archive');
    }
    if (filePositionBig > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new Error('root package.json offset exceeds the safe integer range');
    }

    const contents = readExactly(descriptor, entry.size, Number(filePositionBig));
    try {
      return JSON.parse(contents.toString('utf8'));
    } catch (error) {
      throw new Error(`packed package.json is invalid: ${String(error)}`);
    }
  } finally {
    closeSync(descriptor);
  }
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
}

if (failures.length > 0) {
  for (const message of failures) console.error(`check-packaging: ${message}`);
  process.exit(1);
}
console.error(
  `check-packaging: ok (${String(configAssertions)} config assertions, ` +
    `${String(packs.length)} unpacked pack(s), ${String(packedFilesChecked)} packed files)`,
);
