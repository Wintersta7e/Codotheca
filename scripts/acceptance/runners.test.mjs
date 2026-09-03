import assert from 'node:assert/strict';
import test from 'node:test';

import { parseLibtest, parsePlaywright, parseScriptResults, parseVitest } from './runners.mjs';
import { tagsIn } from './tags.mjs';

test('the tag grammar accepts both spellings and refuses a false positive', () => {
  assert.deepEqual(tagsIn('acceptance_recovery::ac_14_schema_from_the_future'), ['14']);
  assert.deepEqual(tagsIn('AC-45b the rank column contains no node'), ['45b']);
  assert.deepEqual(tagsIn('AC-6 and AC-24 together'), ['6', '24']);
  assert.deepEqual(tagsIn('ac_12balance_is_not_a_tag'), []);
  assert.deepEqual(tagsIn('nothing here'), []);
});

test('parseLibtest reads ok, FAILED and ignored', () => {
  const stdout = [
    'running 3 tests',
    'test acceptance_recovery::ac_14_future ... ok',
    'test acceptance_recovery::ac_14_corrupt ... FAILED',
    'test other::unrelated ... ignored',
    'test result: FAILED. 1 passed; 1 failed; 1 ignored',
  ].join('\n');
  assert.deepEqual(parseLibtest(stdout), [
    { id: 'acceptance_recovery::ac_14_future', status: 'passed', runner: 'cargo', tags: ['14'] },
    { id: 'acceptance_recovery::ac_14_corrupt', status: 'failed', runner: 'cargo', tags: ['14'] },
    { id: 'other::unrelated', status: 'skipped', runner: 'cargo', tags: [] },
  ]);
});

// Regression: `Swatinem/rust-cache` sets CARGO_TERM_COLOR=always, so on CI cargo's own lines
// arrive wrapped in SGR escapes. `Running` then never matched, every integration test was recorded
// under its bare function name, and the harness reported the same eight tests twice — as criteria
// that "did not run" and as results "no check claims". This is that exact stdout.
test('parseLibtest qualifies the binary even when cargo colours its output', () => {
  const E = String.fromCharCode(27);
  const stdout = [
    `${E}[1m${E}[32m     Running${E}[0m tests/acceptance_art.rs (target/debug/deps/acceptance_art-2)`,
    'running 1 test',
    `test ac_56_reroll_offset_is_absolute ... ${E}[32mok${E}[0m`,
  ].join('\n');
  assert.deepEqual(
    parseLibtest(stdout).map((r) => [r.id, r.status]),
    [['acceptance_art::ac_56_reroll_offset_is_absolute', 'passed']],
  );
});

// The shape cargo actually prints. libtest gives a bare function name for an integration test,
// so the binary has to come from cargo's own Running line or the registry could never say which
// file a function lives in — and two files defining `ac_14_a` would be one id.
test('parseLibtest qualifies an integration test with the binary cargo names', () => {
  const stdout = [
    '   Compiling codotheca-core v0.1.0',
    '     Running unittests src/lib.rs (target/debug/deps/codotheca_core-1)',
    'running 1 test',
    'test scan::tests::a_unit_test ... ok',
    '',
    '     Running tests/acceptance_art.rs (target/debug/deps/acceptance_art-2)',
    'running 2 tests',
    'test ac_56_reroll_offset_is_absolute ... ok',
    'test ac_62_fade_is_reference_or_archived_only ... FAILED',
    '',
    '     Running tests/acceptance_storage.rs (target/debug/deps/acceptance_storage-3)',
    'test ac_22_art_cache_under_50mb ... ok',
  ].join('\n');
  assert.deepEqual(
    parseLibtest(stdout).map((r) => [r.id, r.status]),
    [
      ['scan::tests::a_unit_test', 'passed'],
      ['acceptance_art::ac_56_reroll_offset_is_absolute', 'passed'],
      ['acceptance_art::ac_62_fade_is_reference_or_archived_only', 'failed'],
      ['acceptance_storage::ac_22_art_cache_under_50mb', 'passed'],
    ],
  );
});

test('parseVitest reads the jest-compatible json report', () => {
  const report = {
    testResults: [
      {
        name: '/repo/app/test/acceptance/coreRestart.test.ts',
        assertionResults: [
          {
            fullName: 'AC-24 every pending request is rejected with CORE_RESTARTED',
            status: 'passed',
          },
          { fullName: 'AC-24 nothing non-idempotent is replayed', status: 'failed' },
        ],
      },
    ],
  };
  assert.deepEqual(parseVitest(report), [
    {
      id: 'AC-24 every pending request is rejected with CORE_RESTARTED',
      status: 'passed',
      runner: 'vitest',
      tags: ['24'],
    },
    {
      id: 'AC-24 nothing non-idempotent is replayed',
      status: 'failed',
      runner: 'vitest',
      tags: ['24'],
    },
  ]);
});

test('parsePlaywright walks nested suites', () => {
  const report = {
    suites: [
      {
        title: 'acceptance_single_instance.spec.ts',
        specs: [],
        suites: [
          {
            title: 'second launch',
            specs: [
              {
                title: 'AC-13 a second launch opens no window',
                ok: true,
                tests: [{ status: 'expected' }],
              },
            ],
            suites: [],
          },
        ],
      },
    ],
  };
  assert.deepEqual(parsePlaywright(report), [
    { id: 'AC-13 a second launch opens no window', status: 'passed', runner: 'e2e', tags: ['13'] },
  ]);
});

// Playwright reports a `test.skip()` as `ok: true`, because a skip is an expected outcome. Read
// naively that is a pass, and the one spec in this repository that renders a real frame skips
// itself when the release core is absent — so a criterion would read as covered on every machine
// that had not built one. A skip is a skip.
test('parsePlaywright reports a skipped spec as skipped, never as passed', () => {
  const report = {
    suites: [
      {
        title: 'mount.spec.ts',
        specs: [
          {
            title: 'the app paints a real first-run screen',
            ok: true,
            tests: [{ status: 'skipped', results: [{ status: 'skipped' }] }],
          },
        ],
        suites: [],
      },
    ],
  };
  assert.deepEqual(
    parsePlaywright(report).map((r) => [r.id, r.status]),
    [['the app paints a real first-run screen', 'skipped']],
  );
});

test('parseScriptResults refuses a result with no id or an unknown status', () => {
  assert.deepEqual(
    parseScriptResults([{ id: 'check-forbidden:c44-forget-token', status: 'passed' }]),
    [
      {
        id: 'check-forbidden:c44-forget-token',
        status: 'passed',
        runner: 'script',
        tags: [],
      },
    ],
  );
  assert.throws(() => parseScriptResults([{ status: 'passed' }]), /id/u);
  assert.throws(() => parseScriptResults([{ id: 'x', status: 'green' }]), /status/u);
});
