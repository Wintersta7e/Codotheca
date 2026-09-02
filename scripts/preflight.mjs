#!/usr/bin/env node
/**
 * What a build needs before it starts, and the named remedy for each thing it does not have.
 *
 * This exists because the failures it catches name nothing useful when they arrive on their
 * own. One `node_modules` cannot serve two platforms: an install from either side prunes the
 * other's native bindings and writes bin entries the other's shell cannot run, and the symptoms
 * point somewhere else entirely — a missing rollup binding reads as a broken import, a missing
 * bin entry reads as a PATH problem, and an esbuild binary whose version differs from its
 * wrapper reports only `Error: The service was stopped`, which names neither the package nor
 * the version nor the fix.
 *
 * Every problem string starts with a key into `REMEDY`, so a caller can print the command that
 * repairs it rather than the symptom that described it.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

/**
 * Each remedy names a command the reader can run. A remedy that only restates the problem is
 * what made the version-mismatch failure above cost a day.
 */
export const REMEDY = Object.freeze({
  'native-bindings':
    'npm install --no-bin-links && npm rebuild --ignore-scripts && npm run prepare:win',
  'bin-entries': 'npm rebuild --ignore-scripts && npm run prepare:win',
  'esbuild-version':
    'npm rebuild --ignore-scripts && npm run prepare:win — the binary and its wrapper must be the same version',
  'node-version': 'install Node 22.12.0 or newer, then run node scripts/build-dist.mjs again',
  cargo: 'install the Rust toolchain so cargo is on PATH, then run the build again',
  git: 'install git 2.22 or newer, then run node scripts/build-dist.mjs again',
});

/** The rollup binding npm installs for this platform, and nothing else installs. */
function rollupBindings(platform, arch) {
  if (platform === 'win32') return [`rollup-win32-${arch}-msvc`];
  // A musl host installs the musl variant instead; either one satisfies the requirement.
  if (platform === 'linux') return [`rollup-linux-${arch}-gnu`, `rollup-linux-${arch}-musl`];
  return [];
}

function readVersion(packageDir) {
  try {
    const parsed = JSON.parse(readFileSync(join(packageDir, 'package.json'), 'utf8'));
    return typeof parsed.version === 'string' ? parsed.version : null;
  } catch {
    return null;
  }
}

/**
 * The bindings and bin entries this platform needs. An empty array means ready.
 *
 * @param {string} root repository root
 * @param {NodeJS.Platform} [platform]
 * @param {string} [arch]
 * @returns {readonly string[]}
 */
export function checkNativeBindings(root, platform = process.platform, arch = process.arch) {
  const problems = [];
  const modules = join(root, 'node_modules');
  if (!existsSync(modules)) {
    problems.push('native-bindings: node_modules does not exist');
    return problems;
  }

  const candidates = rollupBindings(platform, arch);
  if (
    candidates.length > 0 &&
    !candidates.some((name) => existsSync(join(modules, '@rollup', name)))
  ) {
    problems.push(`native-bindings: none of @rollup/${candidates.join(', @rollup/')} is installed`);
  }

  const esbuildBinding = join(modules, '@esbuild', `${platform}-${arch}`);
  if (!existsSync(esbuildBinding)) {
    problems.push(`native-bindings: @esbuild/${platform}-${arch} is not installed`);
  } else {
    // The binary refuses a wrapper of a different version and says only that its service
    // stopped, so the mismatch is checked here where it can be named.
    const wrapper = readVersion(join(modules, 'esbuild'));
    const binding = readVersion(esbuildBinding);
    if (wrapper !== null && binding !== null && wrapper !== binding) {
      problems.push(
        `esbuild-version: @esbuild/${platform}-${arch} is ${binding} but esbuild is ${wrapper}`,
      );
    }
  }

  // A bare symlink is not runnable from a Windows shell, so the Windows side needs `.cmd`
  // shims and the check differs per platform rather than counting entries.
  const binDir = join(modules, '.bin');
  const entries = existsSync(binDir) ? readdirSync(binDir) : [];
  if (entries.length === 0) {
    problems.push('bin-entries: node_modules/.bin is empty');
  } else if (platform === 'win32' && !entries.some((name) => name.endsWith('.cmd'))) {
    problems.push('bin-entries: node_modules/.bin carries no .cmd shims a Windows shell can run');
  }

  return problems;
}

function meetsNodeFloor(version) {
  const parts = version.replace(/^v/u, '').split('.').map(Number);
  const [major = 0, minor = 0] = parts;
  return major > 22 || (major === 22 && minor >= 12);
}

/** git's floor, mirroring `GIT_FLOOR` in the shared module and the core. */
const GIT_FLOOR = [2, 22];

function toolVersion(command, args) {
  try {
    return execFileSync(command, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
  } catch {
    return null;
  }
}

/**
 * Everything a build needs. An empty array means ready.
 *
 * @param {string} root
 * @returns {readonly string[]}
 */
export function preflight(root) {
  const problems = [...checkNativeBindings(root)];

  if (!meetsNodeFloor(process.version)) {
    problems.push(`node-version: this is Node ${process.version}, and the floor is 22.12.0`);
  }

  if (toolVersion('cargo', ['--version']) === null) {
    problems.push('cargo: cargo is not on PATH, so the core cannot be built');
  }

  const git = toolVersion('git', ['--version']);
  if (git === null) {
    problems.push('git: git is not on PATH');
  } else {
    const found = /(\d+)\.(\d+)/u.exec(git);
    const major = found === null ? 0 : Number(found[1]);
    const minor = found === null ? 0 : Number(found[2]);
    const [floorMajor, floorMinor] = GIT_FLOOR;
    if (major < floorMajor || (major === floorMajor && minor < floorMinor)) {
      problems.push(
        `git: this is git ${String(major)}.${String(minor)}, and the floor is ` +
          `${String(floorMajor)}.${String(floorMinor)}`,
      );
    }
  }

  return problems;
}

/** The remedy for a problem string, keyed by the word before its first colon. */
export function remedyFor(problem) {
  const key = problem.slice(0, problem.indexOf(':'));
  return REMEDY[key] ?? null;
}
