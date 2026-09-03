import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { stripComments, stripRustTestModules, validateForbidden } from '../check-forbidden.mjs';
import { validateCallSites } from '../check-call-sites.mjs';
import { NON_SCAN_PROBLEM_KINDS, evaluateProblemKinds } from '../check-problem-kinds.mjs';
import {
  addObservation,
  evaluateBudget,
  loadMachines,
  newSample,
  percentile,
  resolveMachine,
  summarize,
} from './perf.mjs';

// `fileURLToPath`, never `.pathname`: a file URL's pathname keeps a leading slash, and on
// Windows the drive letter sits after it, so `readFileSync` opens a doubled-drive path.
const machines = loadMachines(
  fileURLToPath(new URL('../../acceptance/machines.json', import.meta.url)),
);
const factsA = { platform: 'win32', cores: 8, memoryGb: 32 };

test('an unclaimed machine resolves to null with a reason, never to a default', () => {
  const resolved = resolveMachine(machines, {}, factsA);
  assert.equal(resolved.id, null);
  assert.match(resolved.reason, /CODOTHECA_REFERENCE_MACHINE/u);
});

test('a claim the hardware contradicts is refused', () => {
  const resolved = resolveMachine(
    machines,
    { CODOTHECA_REFERENCE_MACHINE: 'A' },
    { ...factsA, cores: 4 },
  );
  assert.equal(resolved.id, null);
  assert.match(resolved.reason, /cores/u);
});

test('a claim the hardware supports resolves', () => {
  assert.equal(resolveMachine(machines, { CODOTHECA_REFERENCE_MACHINE: 'A' }, factsA).id, 'A');
});

test('a latency sample with no event pair is refused', () => {
  assert.throws(
    () => newSample({ check: 'AC-16-warm', kind: 'latency', thermal: 'warm' }),
    /from/u,
  );
  assert.throws(
    () =>
      newSample({
        check: 'AC-16-warm',
        kind: 'latency',
        from: 'shortcut callback entry',
        thermal: 'warm',
      }),
    /to/u,
  );
});

test('a rate with no run definition is refused', () => {
  assert.throws(() => newSample({ check: 'AC-30-pacing', kind: 'rate', thermal: 'warm' }), /run/u);
});

test('percentile and summarize report n alongside the figures', () => {
  let sample = newSample({
    check: 'AC-16-warm',
    kind: 'latency',
    from: 'shell global-shortcut callback entry',
    to: 'a composited frame of the window',
    thermal: 'warm',
    machine: 'A',
  });
  for (const v of [8, 9, 10, 40]) sample = addObservation(sample, v);
  assert.equal(percentile([8, 9, 10, 40], 50), 9);
  const s = summarize(sample);
  assert.equal(s.n, 4);
  assert.equal(s.machine, 'A');
  assert.match(s.summary, /p50/u);
});

test('a budget on an unresolved machine skips and never passes', () => {
  const sample = addObservation(
    newSample({
      check: 'AC-16-warm',
      kind: 'latency',
      from: 'a',
      to: 'b',
      thermal: 'warm',
      machine: 'A',
    }),
    1,
  );
  const verdict = evaluateBudget(sample, { metric: 'p95', op: '<', value: 30, machine: 'A' }, null);
  assert.equal(verdict.status, 'skipped');
  assert.notEqual(verdict.status, 'passed');
});

test('a budget for another machine skips rather than being measured here', () => {
  const sample = addObservation(
    newSample({
      check: 'AC-15-cold',
      kind: 'latency',
      from: 'a',
      to: 'b',
      thermal: 'cold',
      machine: 'A',
    }),
    1,
  );
  assert.equal(
    evaluateBudget(sample, { metric: 'p95', op: '<', value: 2500, machine: 'B' }, 'A').status,
    'skipped',
  );
});

test('Reference B carries budgets and no measurements, and says so', () => {
  const b = machines.machines.find((m) => m.id === 'B');
  assert.equal(b.measured, null);
  assert.ok(b.measuredReason.length >= 20);
});

// The carve-outs the static gates make, as unit tests rather than as a property of whichever
// files happen to be in the tree. Both were found by the gates firing on correct code.
test('a comment naming a banned string does not trip the rule it documents', () => {
  const rust = '//! `warm`, `cooling` and `blueprint` are banned.\nlet x = "kept";\n';
  const stripped = stripComments(rust, '.rs');
  assert.ok(!stripped.includes('warm'));
  assert.ok(stripped.includes('"kept"'));
  assert.equal(stripped.split('\n').length, rust.split('\n').length, 'line numbers must survive');
  assert.ok(stripComments('const u = "https://x/y";\n', '.ts').includes('https://x/y'));
});

test('a Rust test module is blanked whichever cfg spelling it carries', () => {
  const short =
    '#[cfg(test)]\nmod tests {\n    let s = "warm";\n}\nfn after() { let k = "kept"; }\n';
  const shortOut = stripRustTestModules(short);
  assert.ok(!shortOut.includes('"warm"'));
  assert.ok(shortOut.includes('"kept"'), 'code after the module must still be scanned');

  // The spelling this codebase actually uses. Matching only `#[cfg(test)]` blanked nothing in
  // the files that matter, and the gate reported a violation against a test asserting the ban.
  const real = '#[cfg(all(test, feature = "testkit"))]\nmod tests {\n    let s = "warm";\n}\n';
  assert.ok(!stripRustTestModules(real).includes('"warm"'));

  // `not(test)` guards production code and must not be blanked.
  const negated = '#[cfg(not(test))]\nfn prod() { let s = "warm"; }\n';
  assert.ok(stripRustTestModules(negated).includes('"warm"'));
});

test('the forbidden rule file and the registry claim each other in both directions', () => {
  const rules = {
    version: 1,
    rules: [{ id: 'x', kind: 'literal', why: 'w'.repeat(30), spec: '§1', targets: ['t'] }],
  };
  const registry = {
    criteria: [
      { id: '44', checks: [{ id: 'AC-44-token', test: 'check-forbidden:c44-forget-token' }] },
    ],
  };
  const problems = validateForbidden(rules, registry);
  assert.ok(problems.some((p) => p.includes('no rule implements it')));
  assert.ok(problems.some((p) => p.includes('no check in criteria.json claims')));
});

test('a call-site rule no check claims is a problem unless it says its entry is pending', () => {
  const registry = { criteria: [] };
  const orphan = {
    rules: [{ id: 'x', roots: ['a'], patterns: ['p'], why: 'w'.repeat(30), allow: [] }],
  };
  assert.ok(
    validateCallSites(orphan, registry).some((p) => p.includes('no check in criteria.json')),
  );

  const pending = {
    rules: [
      {
        id: 'x',
        roots: ['a'],
        patterns: ['p'],
        why: 'w'.repeat(30),
        allow: [],
        pendingRegistryEntry: { criterion: '63' },
      },
    ],
  };
  assert.deepEqual(validateCallSites(pending, registry), []);
});

test('the problem-kind gate names a kind that is stored but not declared, and vice versa', () => {
  const all = ['permission_denied', 'deferred_slow'];
  const clean = evaluateProblemKinds({
    protocol: all,
    storage: ['permission_denied'],
    rust: ['permission_denied'],
    order: all,
    labels: all,
  });
  assert.equal(clean[0].status, 'passed');
  assert.equal(NON_SCAN_PROBLEM_KINDS['deferred_slow'], 'project_job_state');

  const orphaned = evaluateProblemKinds({
    protocol: all,
    storage: ['permission_denied', 'stray'],
    rust: ['permission_denied'],
    order: all,
    labels: all,
  });
  assert.equal(orphaned[0].status, 'failed');
  assert.match(orphaned[0].detail, /does not declare: stray/u);
});
