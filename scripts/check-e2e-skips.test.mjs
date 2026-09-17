import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readdirSync, statSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const checker = join(root, 'scripts', 'check-e2e-skips.mjs');

/** @returns {{status: number, stderr: string}} */
function run(reportPath) {
  try {
    execFileSync(process.execPath, [checker, reportPath], { encoding: 'utf8', stdio: 'pipe' });
    return { status: 0, stderr: '' };
  } catch (err) {
    const e = /** @type {{status: number, stderr: string}} */ (err);
    return { status: e.status, stderr: String(e.stderr ?? '') };
  }
}

function reportWith(specs) {
  const dir = mkdtempSync(join(tmpdir(), 'e2e-skips-'));
  const path = join(dir, 'report.json');
  writeFileSync(path, JSON.stringify({ suites: [{ specs }] }));
  return path;
}

const passing = [
  { title: 'a spec that ran', ok: true, tests: [{ results: [{ status: 'passed' }] }] },
];

/** The newest mtime under `app/e2e`, which is what the checker compares a report against. */
function newestSpecMs() {
  const dir = join(root, 'app', 'e2e');
  return readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isFile())
    .map((e) => statSync(join(dir, e.name)).mtimeMs)
    .reduce((a, b) => Math.max(a, b), 0);
}

test('a report older than the specs it covers is refused, not read', () => {
  const path = reportWith(passing);
  // **Stamped relative to the specs, never to `Date.now()`.** The first version of this test used
  // "an hour ago" and passed only where the specs happened to be newer than that — true in a
  // worktree whose specs were edited today, false in one `git worktree add` stamped at checkout,
  // where every spec is hours old and a one-hour-old report is the *newer* file. It went green
  // in the tree that wrote it and red in the next lane's, which is a test depending on ambient
  // state it does not control. Deriving the instant from the thing under comparison removes it.
  const stale = new Date(newestSpecMs() - 1000);
  utimesSync(path, stale, stale);
  const { status, stderr } = run(path);
  assert.equal(status, 2, 'a stale report must refuse rather than report on the wrong run');
  assert.match(stderr, /predates the specs it claims to cover/u);
});

test('a current report is read, and the spec count is printed', () => {
  const path = reportWith(passing);
  const { status } = run(path);
  assert.equal(status, 0, 'a report newer than the specs is evidence about this tree');
});

test('a skipped spec still fails, which is the check this file exists for', () => {
  const path = reportWith([{ title: 'a spec that skipped', ok: true, tests: [] }]);
  const { status, stderr } = run(path);
  assert.equal(status, 1, 'Playwright exits zero over a skip; this must not');
  assert.match(stderr, /asserted nothing/u);
});

test('a report holding no specs at all is refused', () => {
  const path = reportWith([]);
  const { status, stderr } = run(path);
  // 1, not 2: the script's two codes are "the gate found a problem" (1: no specs, or a skip)
  // and "there is nothing to judge" (2: the report is missing or stale). A report that exists
  // and holds nothing is the first kind. Asserted against the script rather than the other way
  // round — the contract predates this test.
  assert.equal(status, 1, 'a gate whose passing run examined zero specs cannot fail');
  assert.match(stderr, /no specs at all/u);
});

test('a missing report is refused rather than treated as a clean run', () => {
  const dir = mkdtempSync(join(tmpdir(), 'e2e-skips-'));
  const { status, stderr } = run(join(dir, 'absent.json'));
  assert.equal(status, 2);
  assert.match(stderr, /the suite did not run/u);
});
