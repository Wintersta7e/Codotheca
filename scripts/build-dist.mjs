#!/usr/bin/env node
/**
 * One build script per platform, producing the artifacts a release page hands out.
 *
 * It does not cross-compile. A Windows artifact is built on Windows and a Linux artifact on
 * Linux, because the alternative is shipping something that was never executed on the machine
 * it targets. `targetsFor` returns the host's targets and a `--platform` that is not the host
 * is refused by name.
 *
 * Four failure modes shaped this file, and each of them cost a day when it arrived unnamed:
 *
 *  - **One `node_modules` cannot serve two platforms.** `preflight.mjs` runs first and exits
 *    with the command that repairs it.
 *  - **Two toolchains collide in one cargo target directory**, and the symptom is not a
 *    rebuild — it is a link error naming a crate nobody touched, or a compiler crash. The
 *    target directory is set explicitly per platform and never left to the default.
 *  - **A piped command returns the pipe's exit status**, so a failing step reads as a passing
 *    one. Every step is redirected to a file under `dist/logs/` and the file is read afterwards;
 *    nothing here is piped.
 *  - **Killing a child on Windows leaves orphans** holding file locks, and the next build then
 *    fails with an access error naming a file rather than a process. Children are started in
 *    their own group and the group is reaped.
 */
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  createReadStream,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
} from 'node:fs';
import { copyFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { preflight, remedyFor } from './preflight.mjs';

export const DIST_DIR = 'dist';

const HOST_TARGETS = Object.freeze({
  win32: Object.freeze(['nsis', 'portable']),
  linux: Object.freeze(['AppImage', 'deb', 'rpm']),
});

/**
 * The targets built on a given host. Throws for a platform this project does not ship, rather
 * than returning an empty list a caller would read as "nothing to do".
 *
 * @param {NodeJS.Platform} platform
 * @returns {readonly string[]}
 */
export function targetsFor(platform) {
  const targets = HOST_TARGETS[platform];
  if (targets === undefined) {
    throw new Error(`build-dist: ${platform} is not a target of this project`);
  }
  return targets;
}

/** WSL and Windows cargo invalidate each other's artifacts in one directory. */
export function cargoTargetDir(platform) {
  return platform === 'win32' ? 'core/target-win' : 'core/target';
}

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const distDir = join(root, DIST_DIR);
const logDir = join(distDir, 'logs');

function say(line) {
  process.stderr.write(`build-dist: ${line}\n`);
}

/** The last few lines of a step's log, for a failure message that says something. */
function tail(logPath, lines = 25) {
  if (!existsSync(logPath)) return '(no log was written)';
  const text = readFileSync(logPath, 'utf8').split('\n');
  return text.slice(Math.max(0, text.length - lines)).join('\n');
}

let child = null;

function reap() {
  if (child === null || child.exitCode !== null) return;
  // A killed Windows child leaves its grandchildren holding file locks, and the next build
  // fails with an access error naming a file rather than a process.
  if (process.platform === 'win32') {
    spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
  } else {
    try {
      process.kill(-child.pid, 'SIGTERM');
    } catch {
      child.kill('SIGTERM');
    }
  }
}
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    reap();
    process.exit(130);
  });
}

/**
 * Runs one step with its output redirected to a file, and returns only when it has finished.
 * The exit status is the child's own, never a pipeline's.
 */
async function step(name, command, args, env = {}) {
  const logPath = join(logDir, `${name}.log`);
  const { openSync, closeSync } = await import('node:fs');
  const fd = openSync(logPath, 'w');
  say(`${name} …`);
  const started = Date.now();
  const status = await new Promise((resolve, reject) => {
    child = spawn(command, args, {
      cwd: root,
      env: { ...process.env, ...env },
      stdio: ['ignore', fd, fd],
      // Its own group, so the whole tree can be reaped rather than just the parent.
      detached: process.platform !== 'win32',
      shell: false,
    });
    child.on('error', reject);
    child.on('close', resolve);
  }).finally(() => {
    closeSync(fd);
    child = null;
  });
  const seconds = ((Date.now() - started) / 1000).toFixed(1);
  if (status !== 0) {
    say(`${name} FAILED after ${seconds}s (exit ${String(status)}) — ${logPath}`);
    process.stderr.write(`${tail(logPath)}\n`);
    reap();
    process.exit(1);
  }
  say(`${name} ok (${seconds}s)`);
}

const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';

/**
 * The packaging config reads the core out of one directory, and each platform's cargo writes
 * to its own. Copying rather than re-pointing the config keeps a plain `npm run package:dir`
 * working, and the two file names differ so neither platform's copy displaces the other's.
 */
function stageCore(platform) {
  const name = platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core';
  const built = join(root, cargoTargetDir(platform), 'release', name);
  if (!existsSync(built)) {
    say(`the core was not built: ${built}`);
    process.exit(1);
  }
  const staged = join(root, 'core/target/release', name);
  if (built !== staged) {
    mkdirSync(dirname(staged), { recursive: true });
    copyFileSync(built, staged);
  }
  return staged;
}

/**
 * The Windows installer carries a Linux worker per supported architecture (§13). The binary
 * target it needs belongs to another plan; until that lands, this reports which architectures
 * are staged and the pack hook refuses a Windows build that is missing one. That refusal is
 * deliberate — an installer without the worker cannot index those repositories and would fail
 * silently at runtime instead.
 */
function reportWorkers() {
  const staged = [];
  for (const arch of ['x64', 'arm64']) {
    if (existsSync(join(root, 'build/worker', `linux-${arch}`, 'codotheca-worker'))) {
      staged.push(arch);
    }
  }
  say(
    staged.length === 0
      ? 'worker: none staged — stage one per architecture with scripts/stage-worker.mjs (§13)'
      : `worker: staged for ${staged.join(', ')}`,
  );
}

function sha256(path) {
  return new Promise((resolve, reject) => {
    const hash = createHash('sha256');
    createReadStream(path)
      .on('error', reject)
      .on('data', (chunk) => hash.update(chunk))
      .on('end', () => resolve(hash.digest('hex')));
  });
}

/** Sizes are read from a stat, never from a truncated text field. */
async function reportArtifacts() {
  if (!existsSync(distDir)) {
    say('no dist directory was produced');
    return;
  }
  const artifacts = readdirSync(distDir)
    .map((name) => join(distDir, name))
    .filter((path) => statSync(path).isFile());
  if (artifacts.length === 0) {
    say('the build produced no files in dist/');
    process.exit(1);
  }
  say(`${String(artifacts.length)} artifact(s):`);
  for (const path of artifacts) {
    const bytes = statSync(path).size;
    // The sums are what a release page needs, since nothing here verifies a signature.
    process.stderr.write(
      `  ${path.slice(root.length + 1)}  ${String(bytes)} bytes  sha256:${await sha256(path)}\n`,
    );
  }
}

async function main() {
  const args = process.argv.slice(2);
  const platformFlag = args.indexOf('--platform');
  if (platformFlag >= 0) {
    const requested = args[platformFlag + 1];
    if (requested !== process.platform) {
      say(
        `refusing to build for ${String(requested)} on ${process.platform}. This script does ` +
          'not cross-compile: an artifact is built on the platform it runs on, or it ships ' +
          'having never been executed.',
      );
      process.exit(2);
    }
  }

  const targets = targetsFor(process.platform);
  mkdirSync(logDir, { recursive: true });
  say(`host ${process.platform}/${process.arch}, targets ${targets.join(', ')}`);

  const problems = preflight(root);
  if (problems.length > 0) {
    for (const problem of problems) {
      say(problem);
      const remedy = remedyFor(problem);
      if (remedy !== null) say(`  remedy: ${remedy}`);
    }
    process.exit(1);
  }
  say('preflight ok');

  // Generated bindings are gitignored and never travel with a merge, and skipping this
  // surfaces as an unresolved import in code nobody touched.
  await step('gen', npm, ['run', 'gen']);

  await step('cargo-build', 'cargo', ['build', '--release', '--manifest-path', 'core/Cargo.toml'], {
    CARGO_TARGET_DIR: join(root, cargoTargetDir(process.platform)),
  });
  say(`core staged: ${stageCore(process.platform).slice(root.length + 1)}`);
  reportWorkers();

  await step('typecheck', npm, ['run', 'typecheck']);
  await step('lint', npm, ['run', 'lint']);
  await step('build-app', npm, ['run', 'build:app']);
  await step('check-bundle', npm, ['run', 'check:bundle']);

  // The targets themselves live in electron-builder.yml, where `check-packaging.mjs` asserts
  // them; naming them again on the command line would be a second copy of the same list.
  await step('package', process.execPath, [
    join(root, 'scripts/package.mjs'),
    process.platform === 'win32' ? '--win' : '--linux',
  ]);

  await step('check-packaging', process.execPath, [join(root, 'scripts/check-packaging.mjs')]);

  await reportArtifacts();
  say('done');
}

// Only when run as a script; the test imports `targetsFor` without building anything.
if (process.argv[1] !== undefined && process.argv[1].endsWith('build-dist.mjs')) {
  main().catch((error) => {
    say(`failed: ${String(error)}`);
    reap();
    process.exit(1);
  });
}
