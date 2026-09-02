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
