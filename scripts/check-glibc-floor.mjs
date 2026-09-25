#!/usr/bin/env node
/**
 * The Linux floor, read from what ships: the highest glibc every packed ELF binary needs.
 *
 * Both Rust binaries are built on the runner's image, and a binary linked against a newer glibc
 * than the user's system does not start there — with a loader error, before a line of the app
 * runs. Nothing in the release job read that figure back, so the floor a build actually has was
 * whatever the image happened to carry. This reads it from the bytes that are packed.
 *
 * An ELF is a file whose first four bytes are `7f 45 4c 46`. For each, `readelf --version-info`'s
 * version-needs section is read, and the highest `GLIBC_<v>` need whose flags are not `WEAK` is
 * the binary's floor: a weak need is resolved lazily and does not stop the binary loading. A
 * binary with no `GLIBC_` need at all (static, musl) reports `none`.
 *
 * Report mode prints and passes; `--enforce` fails any figure above `--baseline`. Either way it
 * fails when it read no binary, and when `readelf` cannot be run — never a skip.
 */
import { execFileSync } from 'node:child_process';
import { closeSync, lstatSync, openSync, readSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ELF_MAGIC = [0x7f, 0x45, 0x4c, 0x46];
const NEEDS_HEADER = /^Version needs section '\.gnu\.version_r'/u;
const NEED = /^\s*\S+:\s+Name: GLIBC_(\d+(?:\.\d+)*)\s+Flags: (.*?)\s+Version:/u;

/** `2.3.4` → `[2, 3, 4]`, compared component by component: `2.34` is above `2.3.4`. */
function parts(version) {
  return version.split('.').map(Number);
}

function compare(a, b) {
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i += 1) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

/** The highest non-weak `GLIBC_` version-need in `readelf --version-info` text, or `null`. */
export function highestGlibc(readelfText) {
  let inNeeds = false;
  let highest = null;
  for (const line of readelfText.split(/\r?\n/u)) {
    if (NEEDS_HEADER.test(line)) {
      inNeeds = true;
      continue;
    }
    // The section ends at the blank line before the next one.
    if (inNeeds && line.trim() === '') inNeeds = false;
    if (!inNeeds) continue;
    const need = NEED.exec(line);
    if (!need || /\bWEAK\b/u.test(need[2])) continue;
    if (highest === null || compare(need[1], highest) > 0) highest = need[1];
  }
  return highest;
}

/**
 * Whether `path` begins with the ELF magic, or `null` when it vanished before it could be read.
 * The vanish rule of `lib/read-scanned.mjs`, applied to a four-byte read rather than a whole
 * text file: a file gone mid-walk carries nothing and is skipped **before** it is counted.
 */
function isElf(path) {
  let fd;
  try {
    fd = openSync(path, 'r');
  } catch (error) {
    if (error && error.code === 'ENOENT') return null;
    throw error;
  }
  try {
    const head = Buffer.alloc(4);
    const read = readSync(fd, head, 0, 4, 0);
    return read === 4 && ELF_MAGIC.every((byte, i) => head[i] === byte);
  } finally {
    closeSync(fd);
  }
}

/** Every regular file under `dir`. A symlink is not followed: its target is walked where it lives. */
function walk(dir, out) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch (error) {
    if (error && error.code === 'ENOENT') return out;
    throw error;
  }
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) walk(path, out);
    else if (entry.isFile()) out.push(path);
  }
  return out;
}

function defaultReadelf(file) {
  return execFileSync('readelf', ['--version-info', '--wide', file], {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    // The symbol-versions section lists every dynamic symbol; Electron's runs to megabytes.
    maxBuffer: 512 * 1024 * 1024,
  });
}

/**
 * Read every ELF under `dirs`. With `baseline`, `above` lists the binaries whose need exceeds it,
 * and under `enforce` each of them is also a problem.
 */
export function readFloor({ dirs, runReadelf = defaultReadelf, baseline = null, enforce = false }) {
  const binaries = [];
  const problems = [];
  for (const dir of dirs) {
    if (!lstatSync(dir, { throwIfNoEntry: false })?.isDirectory()) {
      problems.push(`${dir}: not a directory`);
      continue;
    }
    for (const file of walk(dir, []).sort()) {
      if (isElf(file) !== true) continue;
      let text;
      try {
        text = runReadelf(file);
      } catch (error) {
        const missing = error && error.code === 'ENOENT';
        problems.push(
          missing
            ? `readelf could not be run (${String(error.message)}); install binutils`
            : `${file}: readelf failed (${String(error.message ?? error)})`,
        );
        if (missing) return { binaries, above: [], problems };
        continue;
      }
      binaries.push({ file, glibc: highestGlibc(text) ?? 'none' });
    }
  }
  if (binaries.length === 0) problems.push(`no ELF binary was read under ${dirs.join(', ')}`);
  const above =
    baseline === null
      ? []
      : binaries.filter((b) => b.glibc !== 'none' && compare(b.glibc, baseline) > 0);
  if (enforce) {
    for (const b of above)
      problems.push(`${b.file} needs GLIBC ${b.glibc}, above the baseline ${baseline}`);
  }
  return { binaries, above, problems };
}

function main(argv) {
  const dirs = [];
  let baseline = null;
  let enforce = false;
  for (let i = 0; i < argv.length; i += 1) {
    const value = argv[i + 1];
    if (argv[i] === '--dir' && value) {
      dirs.push(value);
      i += 1;
    } else if (argv[i] === '--baseline' && value && /^\d+\.\d+$/u.test(value)) {
      baseline = value;
      i += 1;
    } else if (argv[i] === '--enforce') {
      enforce = true;
    } else {
      process.stderr.write(`unexpected argument: ${argv[i]}\n`);
      dirs.length = 0;
      break;
    }
  }
  if (dirs.length === 0 || (enforce && baseline === null)) {
    process.stderr.write(
      'usage: check-glibc-floor.mjs --dir <d>… [--baseline <x.y>] [--enforce]' +
        ' (--enforce needs a baseline)\n',
    );
    return 2;
  }

  const { binaries, above, problems } = readFloor({ dirs, baseline, enforce });
  for (const b of binaries)
    process.stdout.write(`${relative(process.cwd(), b.file)}  GLIBC ${b.glibc}\n`);
  process.stdout.write(`ELF binaries read: ${String(binaries.length)}\n`);
  const needs = binaries.map((b) => b.glibc).filter((g) => g !== 'none');
  if (needs.length > 0) {
    process.stdout.write(
      `highest GLIBC need: ${needs.reduce((a, b) => (compare(a, b) >= 0 ? a : b))}\n`,
    );
  }
  if (!enforce) {
    const list =
      baseline === null
        ? 'not compared'
        : above.map((b) => `${relative(process.cwd(), b.file)} (${b.glibc})`).join(', ') || 'none';
    process.stdout.write(`report mode: baseline ${baseline ?? 'not ruled'}; above it: ${list}\n`);
  }
  for (const p of problems) process.stderr.write(`${p}\n`);
  return problems.length === 0 ? 0 : 1;
}

// `import.meta.url` is not a `file:` URL when a bundler serves this module, and `fileURLToPath`
// throws on anything else — so the scheme is checked before the path is taken.
if (
  process.argv[1] &&
  import.meta.url.startsWith('file:') &&
  fileURLToPath(import.meta.url) === process.argv[1]
)
  process.exit(main(process.argv.slice(2)));
