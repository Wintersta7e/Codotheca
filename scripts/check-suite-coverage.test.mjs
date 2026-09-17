import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const checker = join(root, 'scripts', 'check-suite-coverage.mjs');

/**
 * Both fixtures are always passed, so no case here spawns vitest. Collection is the one thing
 * this gate cannot fake for itself, and a test that waited on it would be measuring the glob
 * rather than the comparison.
 *
 * @returns {{status: number, stdout: string, stderr: string}}
 */
function run(reportPath, collectedPath) {
  try {
    const stdout = execFileSync(process.execPath, [checker, reportPath, collectedPath], {
      encoding: 'utf8',
      stdio: 'pipe',
    });
    return { status: 0, stdout, stderr: '' };
  } catch (err) {
    const e = /** @type {{status: number, stdout: string, stderr: string}} */ (err);
    return { status: e.status, stdout: String(e.stdout ?? ''), stderr: String(e.stderr ?? '') };
  }
}

const dir = mkdtempSync(join(tmpdir(), 'suite-coverage-'));
let seq = 0;

function fixture(name, body) {
  const path = join(dir, `${name}-${String(seq++)}.json`);
  writeFileSync(path, JSON.stringify(body));
  return path;
}

/** What `vitest list --json` emits: one entry per collected test, each naming its file. */
function collected(...files) {
  return fixture(
    'collected',
    files.map((file) => ({ name: 'a test', file, projectName: 'dom' })),
  );
}

/** What `--reporter=json` emits: one entry per file, each holding its assertion results. */
function report(...files) {
  return fixture('report', {
    numTotalTests: files.length,
    numFailedTests: 0,
    testResults: files.map((file) => ({
      name: file,
      assertionResults: [{ fullName: 'a test', status: 'passed' }],
    })),
  });
}

const A = '/repo/app/src/alpha.test.ts';
const B = '/repo/app/src/beta.test.ts';

test('a report holding every collected file passes, and prints the count it scanned', () => {
  const result = run(report(A, B), collected(A, B));
  assert.equal(result.status, 0);
  // §26.2: a gate whose passing run examined zero files cannot fail, so the number is the evidence.
  assert.match(result.stdout, /2 collected file\(s\)/u);
});

test('a file that vanished from the report fails the gate and is named', () => {
  const result = run(report(A), collected(A, B));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /beta\.test\.ts/u);
  assert.doesNotMatch(result.stderr, /alpha\.test\.ts/u);
});

// This is the fault that produced the gate: run 2 of three on `main` exited 1 carrying
// `numFailedTests: 0`, because the lost file was absent rather than failing. A gate keyed on
// failures would have read it as green.
test('a lost file is caught even though the report declares zero failures', () => {
  const result = run(report(A), collected(A, B));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /asserted nothing/u);
});

test('a file present with no assertions counts as lost, not as covered', () => {
  const path = fixture('empty', {
    numFailedTests: 0,
    testResults: [
      { name: A, assertionResults: [{ fullName: 'a test', status: 'passed' }] },
      { name: B, assertionResults: [] },
    ],
  });
  const result = run(path, collected(A, B));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /beta\.test\.ts/u);
});

test('a separator difference is not read as a missing file', () => {
  const result = run(
    report('/repo/app/src/alpha.test.ts'),
    collected('/repo/app/src/alpha.test.ts'),
  );
  assert.equal(result.status, 0);
});

test('an empty collection is a failure, because a glob that matched nothing is not a pass', () => {
  const result = run(report(A), collected());
  assert.equal(result.status, 1);
  assert.match(result.stderr, /collected no test files/u);
});

test('a missing report is refused rather than read as complete', () => {
  const result = run(join(dir, 'does-not-exist.json'), collected(A));
  assert.equal(result.status, 2);
  assert.match(result.stderr, /the suite did not run/u);
});
