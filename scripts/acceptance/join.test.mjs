import assert from 'node:assert/strict';
import test from 'node:test';

import {
  absentRunners,
  diffAgainstBaseline,
  gateProblems,
  joinResults,
  validateBaseline,
} from './join.mjs';

const registry = {
  version: 1,
  criteria: [
    {
      id: '14',
      title: 'recovery',
      group: 'functional',
      spec: '§16.14',
      checks: [
        {
          id: 'AC-14-a',
          status: 'automated',
          runner: 'cargo',
          test: 'r::ac_14_a',
          assert: 'aaaaaaaaaaa',
        },
        {
          id: 'AC-14-b',
          status: 'deferred',
          runner: 'cargo',
          owner: '17',
          test: 'r::ac_14_b',
          assert: 'aaaaaaaaaaa',
        },
      ],
    },
  ],
};

const passed = { id: 'r::ac_14_a', status: 'passed', runner: 'cargo', tags: ['14'] };
const failed = { id: 'r::ac_14_a', status: 'failed', runner: 'cargo', tags: ['14'] };
const skipped = { id: 'r::ac_14_a', status: 'skipped', runner: 'cargo', tags: ['14'] };

test('a joined automated check carries its result', () => {
  const join = joinResults(registry, [passed]);
  const a = join.checks.find((c) => c.id === 'AC-14-a');
  assert.equal(a.result, 'passed');
  const b = join.checks.find((c) => c.id === 'AC-14-b');
  assert.equal(b.result, 'not-run');
});

test('an automated check whose test never ran fails the gate', () => {
  const join = joinResults(registry, []);
  const problems = gateProblems(join, { newFailures: [], stale: [], missing: [] });
  assert.ok(problems.some((p) => p.includes('AC-14-a') && p.includes('did not run')));
});

// A skip reads as green in every runner's own summary. It is the honest thing for a spec to do
// when its prerequisite is missing and the dishonest thing for a criterion to inherit.
test('an automated check whose test skipped fails the gate rather than passing', () => {
  const join = joinResults(registry, [skipped]);
  const problems = gateProblems(join, { newFailures: [], stale: [], missing: [] }, [skipped]);
  assert.ok(problems.some((p) => p.includes('AC-14-a') && p.includes('skip is not a pass')));
});

test('a suite that did not run at all is named, not silently forgiven', () => {
  const join = joinResults(registry, []);
  const shellOnly = [{ id: 'x', status: 'passed', runner: 'vitest', tags: [] }];
  assert.deepEqual(absentRunners(join, shellOnly), ['cargo']);
  const problems = gateProblems(join, { newFailures: [], stale: [], missing: [] }, shellOnly);
  assert.deepEqual(problems, []);
});

test('an unlisted failure is a regression', () => {
  const diff = diffAgainstBaseline([failed], { knownRed: [] });
  assert.deepEqual(diff.newFailures, ['r::ac_14_a']);
});

test('a baseline entry that now passes is stale', () => {
  const baseline = {
    knownRed: [{ test: 'r::ac_14_a', criterion: '14', owner: '17', reason: 'x'.repeat(25) }],
  };
  const diff = diffAgainstBaseline([passed], baseline);
  assert.deepEqual(diff.stale, ['r::ac_14_a']);
  assert.deepEqual(diff.newFailures, []);
});

test('a baseline entry whose test did not run is rot', () => {
  const baseline = {
    knownRed: [{ test: 'r::gone', criterion: '14', owner: '17', reason: 'x'.repeat(25) }],
  };
  assert.deepEqual(diffAgainstBaseline([passed], baseline).missing, ['r::gone']);
});

test('a baseline entry must carry an owner and a reason and name a real criterion', () => {
  const bad = { knownRed: [{ test: 'r::x', criterion: '99', reason: 'short' }] };
  const problems = validateBaseline(bad, registry);
  assert.ok(problems.some((p) => p.includes('owner')));
  assert.ok(problems.some((p) => p.includes('reason')));
  assert.ok(problems.some((p) => p.includes('99')));
});

// The second copy of `OWNER`. One value stated twice, in two files, in one language — R24 — and
// `join.mjs`'s is the one a reader misses, so it is read from this side too.
test('a baseline row may be owned by a phase-3 plan id', () => {
  const row = (owner) => ({
    knownRed: [{ test: 'r::ac_14_a', criterion: '14', owner, reason: 'x'.repeat(25) }],
  });
  for (const owner of ['17', '13c', 'p2-20', 'p3-33', 'p3-36a', 'p4-44', 'p4-L0b']) {
    assert.deepEqual(validateBaseline(row(owner), registry), [], owner);
  }
  // [p4] R218: the unheld phase moves to phase 5, never deleted.
  assert.ok(validateBaseline(row('p5-33'), registry).some((p) => p.includes('owner')));
});

test('a tagged test no check claims fails the gate', () => {
  const orphan = {
    id: 'AC-99 nobody claims this',
    status: 'passed',
    runner: 'vitest',
    tags: ['99'],
  };
  const join = joinResults(registry, [passed, orphan]);
  const problems = gateProblems(join, { newFailures: [], stale: [], missing: [] }, [
    passed,
    orphan,
  ]);
  assert.ok(problems.some((p) => p.includes('no check claims it') && p.includes('AC-99')));
});
