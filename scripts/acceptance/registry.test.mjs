import assert from 'node:assert/strict';
import test from 'node:test';

import { criterionOf, loadRegistry, rollUp, validateRegistry } from './registry.mjs';

const registryPath = new URL('../../acceptance/criteria.json', import.meta.url).pathname;

const entry = (over = {}) => ({
  id: '1',
  title: 'Every repository appears exactly once',
  group: 'functional',
  spec: '§16.1',
  checks: [
    {
      id: 'AC-1-once',
      status: 'deferred',
      runner: 'cargo',
      owner: '07',
      test: 'x::y',
      assert: 'a'.repeat(12),
    },
  ],
  ...over,
});

test('criterionOf reads the criterion out of a check id', () => {
  assert.equal(criterionOf('AC-14'), '14');
  assert.equal(criterionOf('AC-45b-grid-and-hero'), '45b');
  assert.equal(criterionOf('AC-8-live'), '8');
  assert.equal(criterionOf('nonsense'), null);
});

test('a deferred check must name the plan that owns it', () => {
  const bad = entry({
    checks: [
      {
        id: 'AC-1-once',
        status: 'deferred',
        runner: 'cargo',
        test: 'x::y',
        assert: 'a'.repeat(12),
      },
    ],
  });
  const problems = validateRegistry({ version: 1, criteria: [bad] });
  assert.ok(problems.some((p) => p.includes('AC-1-once') && p.includes('owner')));
});

test('a manual check must name a gate and give a reason', () => {
  const bad = entry({
    checks: [{ id: 'AC-1-once', status: 'manual', runner: 'manual', assert: 'a'.repeat(12) }],
  });
  const problems = validateRegistry({ version: 1, criteria: [bad] });
  assert.ok(problems.some((p) => p.includes('gate')));
  assert.ok(problems.some((p) => p.includes('reason')));
});

test('external is criterion 29 and nothing else', () => {
  const wrong = entry({
    checks: [
      {
        id: 'AC-1-x',
        status: 'external',
        runner: 'none',
        reason: 'a'.repeat(30),
        assert: 'a'.repeat(12),
      },
    ],
  });
  assert.ok(validateRegistry({ version: 1, criteria: [wrong] }).some((p) => p.includes('29')));
});

test('rollUp takes the weakest check', () => {
  assert.equal(
    rollUp({
      checks: [{ status: 'automated' }, { status: 'manual' }, { status: 'deferred' }],
    }),
    'manual',
  );
});

test('the shipped registry validates and covers 1..67', () => {
  const registry = loadRegistry(registryPath);
  assert.deepEqual(validateRegistry(registry), []);
  const integers = new Set(registry.criteria.map((c) => Number.parseInt(c.id, 10)));
  for (let n = 1; n <= 67; n += 1) assert.ok(integers.has(n), `criterion ${String(n)} is missing`);
  assert.equal(integers.size, 67);
});

test('every performance check states an event pair or is unmeasurable', () => {
  const registry = loadRegistry(registryPath);
  const perf = registry.criteria.filter((c) => c.group === 'performance');
  assert.deepEqual(
    perf.map((c) => c.id).sort(),
    ['15', '16', '17', '18', '19', '20', '21', '22', '30', '31'].sort(),
  );
  for (const entry of perf) {
    for (const check of entry.checks) {
      if (check.status === 'unmeasurable') {
        assert.ok(check.reason.length >= 20, `${check.id} needs a reason`);
        assert.equal(check.budget, undefined, `${check.id} may not carry a budget`);
        continue;
      }
      // A check that states no number needs no event pair: requiring one would be met by
      // inventing one, which is the failure the rule exists to prevent.
      if (check.runner !== 'perf' && (check.budget ?? []).length === 0) {
        assert.equal(check.measurement, undefined, `${check.id} states no number and no pair`);
        continue;
      }
      const m = check.measurement;
      assert.ok(m, `${check.id} has no measurement`);
      if (m.kind === 'rate') assert.ok(m.run.length > 0, `${check.id} needs a run definition`);
      else if (m.kind !== 'size') {
        assert.ok(m.from.length > 0, `${check.id} needs a from-event`);
        assert.ok(m.to.length > 0, `${check.id} needs a to-event`);
      }
    }
  }
});

test('nothing on the perf runner is automated, and an automated perf check measures a size', () => {
  const registry = loadRegistry(registryPath);
  for (const entry of registry.criteria.filter((c) => c.group === 'performance')) {
    for (const check of entry.checks) {
      if (check.runner === 'perf') {
        assert.notEqual(check.status, 'automated', check.id);
        continue;
      }
      if (check.status !== 'automated') continue;
      // A latency, duration or rate is meaningless without the machine that produced it, and CI
      // is neither reference machine. A size is not: bytes on disk do not vary with the CPU.
      assert.equal(check.runner, 'cargo', `${check.id} runs somewhere a size can be measured`);
      assert.equal(check.measurement.kind, 'size', `${check.id} measures a size, not a time`);
    }
  }
});

test('criterion 29 is external, alone, and carries its reason', () => {
  const registry = loadRegistry(registryPath);
  const external = registry.criteria.filter((c) => c.checks.some((k) => k.status === 'external'));
  assert.deepEqual(
    external.map((c) => c.id),
    ['29'],
  );
  const entry = external[0];
  assert.equal(rollUp(entry), 'external');
  for (const check of entry.checks) {
    assert.match(check.reason, /beta/iu);
    assert.equal(check.runner, 'none');
  }
});

test('the honesty criteria each carry at least one automated check today', () => {
  const registry = loadRegistry(registryPath);
  for (const id of ['23', '24', '25']) {
    const entry = registry.criteria.find((c) => c.id === id);
    assert.ok(entry, `criterion ${id} is missing`);
    assert.ok(
      entry.checks.some((k) => k.status === 'automated'),
      `criterion ${id} has nothing running today`,
    );
  }
});

test('45 and 48 exist only in lettered form and 46 does not', () => {
  const registry = loadRegistry(registryPath);
  const ids = new Set(registry.criteria.map((c) => c.id));
  for (const id of ['45a', '45b', '45c', '48a', '48b']) assert.ok(ids.has(id), `${id} is missing`);
  for (const id of ['45', '48', '46a', '46b']) assert.ok(!ids.has(id), `${id} must not exist`);
});

test('the greppable halves are automated static gates', () => {
  const registry = loadRegistry(registryPath);
  for (const id of ['38', '39', '40', '44', '46']) {
    const entry = registry.criteria.find((c) => c.id === id);
    assert.ok(
      entry.checks.some((k) => k.status === 'automated' && k.runner === 'script'),
      `criterion ${id} has no static gate`,
    );
  }
});

test('every check id is unique across the whole registry', () => {
  const registry = loadRegistry(registryPath);
  const ids = registry.criteria.flatMap((c) => c.checks.map((k) => k.id));
  assert.equal(new Set(ids).size, ids.length);
});

test('every deferred check names a plan that exists in the plan set', () => {
  const registry = loadRegistry(registryPath);
  // The plan set is a literal, not a directory listing: the plans live under a gitignored
  // directory, so a test that read them would scan nothing on a fresh clone and pass on nothing.
  // R44: the second halves own the surfaces most of these criteria test, so they are plans too.
  const plans = new Set([
    '01',
    '02',
    '03',
    '04',
    '05',
    '06',
    '07',
    '08',
    '09',
    '10',
    '10b',
    '11',
    '11b',
    '11c',
    '12',
    '12b',
    '12c',
    '13',
    '13b',
    '13c',
    '14',
    '14b',
    '15',
    '15b',
    '16',
    '16b',
    '16c',
    '17',
    '17b',
    '17c',
    '18',
    '19',
    '20',
    '20b',
    '21',
    '22',
    '23',
    '24',
  ]);
  for (const entry of registry.criteria) {
    for (const check of entry.checks) {
      if (check.status !== 'deferred') continue;
      assert.ok(plans.has(check.owner), `${check.id} names plan ${String(check.owner)}`);
    }
  }
});

test('a check that carries a reason has one worth reading', () => {
  const registry = loadRegistry(registryPath);
  for (const entry of registry.criteria) {
    for (const check of entry.checks) {
      if (check.reason === undefined) continue;
      assert.ok(check.reason.length >= 20, `${check.id}'s reason says nothing`);
    }
  }
});

test('a budget anywhere in the registry names the measurement it is a budget for', () => {
  const registry = loadRegistry(registryPath);
  for (const entry of registry.criteria) {
    for (const check of entry.checks) {
      if ((check.budget ?? []).length === 0) continue;
      assert.ok(check.measurement, `${check.id} carries a budget and no measurement`);
    }
  }
});
