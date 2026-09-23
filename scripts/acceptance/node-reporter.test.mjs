/**
 * The `node:test` capture. The harness and the protocol suite report to their own logs only, so a
 * criterion whose test lives there could not be registered and a tag in one of their names never
 * reached the untagged net — the blindness R127.5 describes, one runner at a time.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { joinResults } from './join.mjs';
import { parseNodeTest } from './runners.mjs';

const root = fileURLToPath(new URL('../..', import.meta.url));
const reporter = join(root, 'scripts/acceptance/node-reporter.mjs');

/** Run `node --test` over one planted file with the capture reporter, and read the capture. */
function capture(source) {
  const dir = mkdtempSync(join(tmpdir(), 'node-capture-'));
  try {
    const file = join(dir, 'planted.test.mjs');
    writeFileSync(file, source);
    const out = join(dir, 'node.json');
    // A `node --test` started inside a test file inherits NODE_TEST_CONTEXT and reports to its
    // parent runner instead of to the reporter it was given, writing no capture at all.
    const env = { ...process.env, NODE_OPTIONS: '' };
    delete env.NODE_TEST_CONTEXT;
    const run = spawnSync(
      process.execPath,
      ['--test', `--test-reporter=${reporter}`, `--test-reporter-destination=${out}`, file],
      { encoding: 'utf8', env },
    );
    return { run, rows: JSON.parse(readFileSync(out, 'utf8')), file };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('the reporter writes one row per test, keyed file :: full name, with its status', () => {
  const { rows } = capture(
    [
      "import test, { describe, it } from 'node:test';",
      "test('a top-level pass', () => {});",
      "test('a failure', () => { throw new Error('no'); });",
      "test('a skip', { skip: true }, () => {});",
      "describe('an outer suite', () => { it('an inner case', () => {}); });",
    ].join('\n'),
  );
  const byName = new Map(rows.map((r) => [r.id.slice(r.id.indexOf('::') + 2), r.status]));
  assert.equal(byName.get('a top-level pass'), 'passed');
  assert.equal(byName.get('a failure'), 'failed');
  assert.equal(byName.get('a skip'), 'skipped');
  assert.equal(byName.get('an outer suite an inner case'), 'passed');
  assert.ok(!byName.has('an outer suite'), 'a suite is not a test');
  for (const row of rows) assert.match(row.id, /planted\.test\.mjs::/u);
});

test('parseNodeTest reads the capture as runner node, with tags taken from the name', () => {
  const results = parseNodeTest([
    { id: 'scripts/x.test.mjs::ac_p3_34_15 no assert states the count', status: 'passed' },
    { id: 'scripts/x.test.mjs::untagged', status: 'failed' },
  ]);
  assert.deepEqual(
    results.map((r) => [r.runner, r.status, r.tags]),
    [
      ['node', 'passed', ['P3-34-15']],
      ['node', 'failed', []],
    ],
  );
  assert.throws(() => parseNodeTest({}), /array/u);
});

// The bite: an unregistered, tagged node:test test must now be seen by the untagged net.
test('a tagged node:test test no check claims is reported as untagged', () => {
  const { rows } = capture(
    "import test from 'node:test';\ntest('ac_p3_34_15 an orphan the register does not name', () => {});\n",
  );
  const registry = { criteria: [{ id: 'x', checks: [] }] };
  const joined = joinResults(registry, parseNodeTest(rows));
  assert.equal(joined.untagged.length, 1, JSON.stringify(rows));
  assert.deepEqual(joined.untagged[0].tags, ['P3-34-15']);
});
