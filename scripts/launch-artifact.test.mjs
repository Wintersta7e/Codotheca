import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { launch } from './launch-artifact.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const cli = join(root, 'scripts', 'launch-artifact.mjs');

/**
 * A stand-in for the packaged app: it writes the shell's `artifact:` line and an index into its
 * data directory, as the real shell and core do, and quits on SIGTERM. `FAKE_MODE` makes it fail
 * each way a real launch can.
 */
const FAKE = `
import { appendFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
const mode = process.env.FAKE_MODE ?? 'ok';
const dataDir = process.env.CODOTHECA_DATA_DIR || join(process.env.FAKE_DEFAULT_PARENT, 'Codotheca');
if (mode === 'exit-early') process.exit(3);
mkdirSync(join(dataDir, 'logs'), { recursive: true });
const log = join(dataDir, 'logs', 'codotheca.log');
appendFileSync(log, new Date().toISOString() + ' info shell artifact: 0.9.0 · appimage\\n');
if (mode === 'crash-loop') {
  appendFileSync(log, new Date().toISOString() + ' error shell core failed (crash_loop): two crashes\\n');
}
if (mode !== 'no-index') writeFileSync(join(dataDir, 'index.db'), '');
process.on('SIGTERM', () => process.exit(0));
setInterval(() => undefined, 1000);
`;

const fakeDir = mkdtempSync(join(tmpdir(), 'launch-fake-'));
const fake = join(fakeDir, 'fake-app.mjs');
writeFileSync(fake, FAKE);

const fast = { timeoutMs: 8000, settleMs: 300, pollMs: 50, graceMs: 2000 };

function start(mode, over = {}) {
  return launch({
    kind: 'appimage',
    exe: process.execPath,
    args: [fake],
    dataDir: mkdtempSync(join(tmpdir(), 'launch-data-')),
    env: { FAKE_MODE: mode },
    ...fast,
    ...over,
  });
}

// The fake quits on SIGTERM; on Windows the script asks a *window* to close, which a console
// process has none of, so these run where the signal is the quit.
const skipReason =
  process.platform === 'win32'
    ? 'not run: the fake app quits on SIGTERM, not a window close'
    : null;

test('AC-P4-48-3 a launch that writes the log line and the index is reported launched', async (t) => {
  if (skipReason !== null) return t.skip(skipReason);
  const result = await start('ok');
  assert.equal(result.kind, 'appimage');
  assert.ok(result.logLines >= 1, 'the artifact line was read');
  assert.ok(existsSync(result.indexPath), 'the index the core opened is there');
  assert.equal(result.quit, 'clean', 'the app quit when asked');
});

test('AC-P4-48-3 an app that exits before its log line fails, naming the exit code', async (t) => {
  if (skipReason !== null) return t.skip(skipReason);
  await assert.rejects(start('exit-early'), /exited with code 3/u);
});

test('AC-P4-48-3 a log naming crash_loop fails', async (t) => {
  if (skipReason !== null) return t.skip(skipReason);
  await assert.rejects(start('crash-loop'), /crash_loop/u);
});

test('AC-P4-48-3 with no data directory given, the one the app wrote is found', async (t) => {
  if (skipReason !== null) return t.skip(skipReason);
  const parent = mkdtempSync(join(tmpdir(), 'launch-default-'));
  const result = await start('ok', {
    dataDir: null,
    defaultDataDirParent: parent,
    env: { FAKE_MODE: 'ok', FAKE_DEFAULT_PARENT: parent },
  });
  assert.equal(result.dataDir, join(parent, 'Codotheca'));
});

/** @returns {{status: number, stdout: string}} */
function runCli(args) {
  try {
    return {
      status: 0,
      stdout: execFileSync(process.execPath, [cli, ...args], { encoding: 'utf8' }),
    };
  } catch (err) {
    const e = /** @type {{status: number, stdout: string}} */ (err);
    return { status: e.status, stdout: String(e.stdout ?? '') };
  }
}

test('AC-P4-48-3 a ledger with zero launches fails the count step', () => {
  const dir = mkdtempSync(join(tmpdir(), 'launch-ledger-'));
  const ledger = join(dir, 'launches.jsonl');
  writeFileSync(ledger, '');
  const empty = runCli(['--count-ledger', ledger]);
  assert.equal(empty.status, 1);
  assert.match(empty.stdout, /^launches recorded: 0$/mu);

  writeFileSync(ledger, `${JSON.stringify({ kind: 'deb', quit: 'clean' })}\n`);
  const one = runCli(['--count-ledger', ledger]);
  assert.equal(one.status, 0, one.stdout);
  assert.match(one.stdout, /^launches recorded: 1 \(deb\)$/mu);
});
