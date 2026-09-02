import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { collectResults } from '../acceptance.mjs';

test('collectResults reads every runner file in the directory', () => {
  const dir = mkdtempSync(join(tmpdir(), 'codotheca-acceptance-'));
  writeFileSync(join(dir, 'cargo.txt'), 'test r::ac_14_a ... ok\n');
  writeFileSync(
    join(dir, 'vitest.json'),
    JSON.stringify({
      testResults: [{ assertionResults: [{ fullName: 'AC-24 x', status: 'failed' }] }],
    }),
  );
  writeFileSync(
    join(dir, 'e2e.json'),
    JSON.stringify({
      suites: [
        { specs: [{ title: 'AC-13 y', ok: true, tests: [{ status: 'expected' }] }], suites: [] },
      ],
    }),
  );
  writeFileSync(
    join(dir, 'script-forbidden.json'),
    JSON.stringify([{ id: 'check-forbidden:c44-forget-token', status: 'passed' }]),
  );

  const results = collectResults(dir);
  assert.deepEqual(
    results.map((r) => [r.runner, r.id, r.status]).sort(),
    [
      ['cargo', 'r::ac_14_a', 'passed'],
      ['e2e', 'AC-13 y', 'passed'],
      ['script', 'check-forbidden:c44-forget-token', 'passed'],
      ['vitest', 'AC-24 x', 'failed'],
    ].sort(),
  );
});

test('collectResults on an empty directory returns nothing rather than throwing', () => {
  const dir = mkdtempSync(join(tmpdir(), 'codotheca-acceptance-'));
  assert.deepEqual(collectResults(dir), []);
});

test('collectResults on a directory that does not exist returns nothing', () => {
  assert.deepEqual(collectResults(join(tmpdir(), 'codotheca-acceptance-no-such-dir')), []);
});
