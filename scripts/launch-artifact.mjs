#!/usr/bin/env node
/**
 * Launch one packaged artifact and prove it started — then ask it to quit.
 *
 * The release workflow checked that artifacts existed and never ran one. *Launched* here means,
 * within the timeout: the rolling log in the data directory (`<dataDir>/logs/codotheca.log`)
 * holds the shell's `artifact:` line written by this run, the index file exists in the data
 * directory (the core started and opened it), and nothing this run logged names `crash_loop` or
 * `StdinEof`. It then waits a settle period and reads the log again, because a core that dies at
 * once crash-loops a moment after the shell's first line, behind a window that looks alive.
 *
 * Then the app is asked to quit — SIGTERM to its process group on Linux; `taskkill /T` without
 * `/F` on Windows, which closes the window, and closing the window quits — and the exit is
 * awaited. A process tree still alive after the grace period is killed and reported as `killed`,
 * never hidden.
 *
 * With no data directory given, the app's default one is found under `--default-data-dir`: the
 * child directory whose log this run wrote. That is how an installed and a portable build are
 * compared — each launch records the directory it actually wrote.
 */
import { execFile, spawn } from 'node:child_process';
import { appendFileSync, existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const KINDS = ['appimage', 'deb', 'nsis', 'portable'];
const LOG = join('logs', 'codotheca.log');
const INDEX = 'index.db';
const FAILURE = /crash_loop|StdinEof/u;
const ARTIFACT_LINE = / artifact: /u;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

function sizeOf(path) {
  try {
    return statSync(path).size;
  } catch {
    return 0;
  }
}

/** What this run appended to a log, given its size before the launch. */
function readSince(path, offset) {
  let text;
  try {
    text = readFileSync(path, 'utf8');
  } catch {
    return '';
  }
  // A log smaller than it was has rolled, and everything in it is this run's.
  return Buffer.byteLength(text) >= offset ? Buffer.from(text).subarray(offset).toString() : text;
}

function childDirs(parent) {
  try {
    return readdirSync(parent, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => join(parent, e.name));
  } catch {
    return [];
  }
}

function alive(pid, group) {
  try {
    process.kill(group ? -pid : pid, 0);
    return true;
  } catch {
    return false;
  }
}

function killTree(child, signal) {
  if (child.pid === undefined) return;
  if (process.platform === 'win32') {
    const args = ['/PID', String(child.pid), '/T'];
    if (signal === 'SIGKILL') args.push('/F');
    execFile('taskkill', args, () => undefined);
    return;
  }
  try {
    process.kill(-child.pid, signal);
  } catch {
    // The group is already gone.
  }
}

/**
 * Launch `exe` and resolve once it has provably started and been asked to quit; reject naming
 * what was missing, the exit code, or the failure the log named.
 */
export async function launch({
  kind,
  exe,
  args = [],
  dataDir = null,
  defaultDataDirParent = null,
  timeoutMs,
  env = {},
  settleMs = 15_000,
  pollMs = 500,
  graceMs = 20_000,
}) {
  if (dataDir === null && defaultDataDirParent === null) {
    throw new Error('launch needs a data directory or the parent of the default one');
  }
  const candidates = () => (dataDir !== null ? [dataDir] : childDirs(defaultDataDirParent));
  const offsets = new Map(candidates().map((dir) => [dir, sizeOf(join(dir, LOG))]));

  const childEnv = { ...process.env, ...env };
  if (dataDir !== null) childEnv.CODOTHECA_DATA_DIR = dataDir;
  else delete childEnv.CODOTHECA_DATA_DIR;

  const group = process.platform !== 'win32';
  const child = spawn(exe, args, {
    env: childEnv,
    detached: group,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  const keep = (chunk) => {
    output = (output + String(chunk)).slice(-8192);
  };
  child.stdout.on('data', keep);
  child.stderr.on('data', keep);
  let exit = null;
  const exited = new Promise((resolve) => {
    child.on('exit', (code, signal) => {
      exit = { code, signal };
      resolve();
    });
    child.on('error', (error) => {
      exit = { code: null, signal: null, error };
      resolve();
    });
  });

  const failWith = (message) => {
    killTree(child, 'SIGKILL');
    const error = new Error(`${kind}: ${message}`);
    error.output = output;
    return error;
  };
  const exitText = () =>
    exit.error !== undefined
      ? `could not be started (${String(exit.error.message)})`
      : `exited with code ${String(exit.code)}${exit.signal ? ` (signal ${exit.signal})` : ''}`;

  /** The data directory this run wrote, once it holds the artifact line and the index. */
  const probe = () => {
    for (const dir of candidates()) {
      const text = readSince(join(dir, LOG), offsets.get(dir) ?? 0);
      if (text === '') continue;
      const failure = FAILURE.exec(text);
      if (failure) throw failWith(`the log in ${dir} names ${failure[0]}`);
      if (ARTIFACT_LINE.test(text) && existsSync(join(dir, INDEX))) return { dir, text };
    }
    return null;
  };

  const deadline = Date.now() + timeoutMs;
  let found = null;
  while (found === null) {
    if (exit !== null) throw failWith(`${exe} ${exitText()} before it launched`);
    found = probe();
    if (found !== null) break;
    if (Date.now() > deadline) {
      throw failWith(`${exe} wrote no artifact line and index within ${String(timeoutMs)} ms`);
    }
    await sleep(pollMs);
  }

  await Promise.race([exited, sleep(settleMs)]);
  if (exit !== null) throw failWith(`${exe} ${exitText()} after it started`);
  const settled = readSince(join(found.dir, LOG), offsets.get(found.dir) ?? 0);
  const failure = FAILURE.exec(settled);
  if (failure) throw failWith(`the log in ${found.dir} names ${failure[0]}`);

  killTree(child, 'SIGTERM');
  await Promise.race([exited, sleep(graceMs)]);
  let quit = 'clean';
  // The first process gone is not the tree gone: on Linux the whole group must have exited.
  const lingering = () => exit === null || (group && alive(child.pid, true));
  const until = Date.now() + graceMs;
  while (lingering() && Date.now() < until) await sleep(pollMs);
  if (lingering()) {
    killTree(child, 'SIGKILL');
    quit = 'killed';
    await Promise.race([exited, sleep(graceMs)]);
  }

  return {
    kind,
    exe,
    pid: child.pid,
    dataDir: found.dir,
    logLines: settled.split('\n').filter((line) => line.trim() !== '').length,
    indexPath: join(found.dir, INDEX),
    quit,
  };
}

function countLedger(path) {
  let lines = [];
  try {
    lines = readFileSync(path, 'utf8')
      .split('\n')
      .filter((line) => line.trim() !== '');
  } catch {
    // A ledger nothing wrote is a ledger of zero launches.
  }
  const kinds = [];
  for (const line of lines) {
    try {
      kinds.push(String(JSON.parse(line).kind));
    } catch {
      process.stderr.write(`unreadable ledger line: ${line}\n`);
      return 1;
    }
  }
  process.stdout.write(
    `launches recorded: ${String(kinds.length)}${kinds.length > 0 ? ` (${kinds.join(', ')})` : ''}\n`,
  );
  return kinds.length > 0 ? 0 : 1;
}

const USAGE =
  'usage: launch-artifact.mjs --kind <appimage|deb|nsis|portable> --exe <path>' +
  ' [--data-dir <d> | --default-data-dir <parent>] [--timeout-s 90] [--ledger <file>]' +
  ' [-- <app args>]\n       launch-artifact.mjs --count-ledger <file>\n';

async function main(argv) {
  const split = argv.indexOf('--');
  const own = split === -1 ? argv : argv.slice(0, split);
  const appArgs = split === -1 ? [] : argv.slice(split + 1);
  const opts = {};
  for (let i = 0; i < own.length; i += 2) {
    const value = own[i + 1];
    if (!own[i].startsWith('--') || value === undefined || value === '') {
      process.stderr.write(USAGE);
      return 2;
    }
    opts[own[i].slice(2)] = value;
  }
  if (opts['count-ledger'] !== undefined) return countLedger(opts['count-ledger']);

  const dataDir = opts['data-dir'] ?? null;
  const parent = opts['default-data-dir'] ?? null;
  const timeoutS = Number(opts['timeout-s'] ?? '90');
  if (
    !KINDS.includes(opts.kind) ||
    opts.exe === undefined ||
    (dataDir === null) === (parent === null) ||
    !Number.isFinite(timeoutS)
  ) {
    process.stderr.write(USAGE);
    return 2;
  }

  const attempt = (args) =>
    launch({
      kind: opts.kind,
      exe: opts.exe,
      args,
      dataDir,
      defaultDataDirParent: parent,
      timeoutMs: timeoutS * 1000,
    });
  let result;
  try {
    result = await attempt(appArgs);
  } catch (error) {
    // An extracted AppImage cannot use Chromium's SUID sandbox helper, and on a runner that also
    // restricts user namespaces it dies naming it. Only then, and only for that artifact, the
    // launch is retried without the sandbox — and the output says so.
    const sandbox = /SUID sandbox/u.exec(String(error.output ?? ''));
    if (opts.kind !== 'appimage' || sandbox === null || appArgs.includes('--no-sandbox')) {
      process.stderr.write(`${String(error.message)}\n${String(error.output ?? '')}\n`);
      return 1;
    }
    process.stdout.write('--no-sandbox passed to appimage: it died on the SUID sandbox helper\n');
    try {
      result = await attempt([...appArgs, '--no-sandbox']);
    } catch (retry) {
      process.stderr.write(`${String(retry.message)}\n${String(retry.output ?? '')}\n`);
      return 1;
    }
  }

  process.stdout.write(
    `launched ${result.kind}: ${result.exe} pid ${String(result.pid)}\n` +
      `data directory: ${result.dataDir}\n` +
      `log lines this run: ${String(result.logLines)}; index: ${result.indexPath}\n` +
      `quit: ${result.quit}${result.quit === 'killed' ? ' — the app did not exit when asked' : ''}\n`,
  );
  if (opts.ledger !== undefined) appendFileSync(opts.ledger, `${JSON.stringify(result)}\n`);
  return 0;
}

// `import.meta.url` is not a `file:` URL when a bundler serves this module, and `fileURLToPath`
// throws on anything else — so the scheme is checked before the path is taken.
if (
  process.argv[1] &&
  import.meta.url.startsWith('file:') &&
  fileURLToPath(import.meta.url) === process.argv[1]
)
  process.exit(await main(process.argv.slice(2)));
