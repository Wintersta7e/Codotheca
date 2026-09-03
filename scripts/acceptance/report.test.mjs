import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { joinResults } from './join.mjs';
import { loadRegistry } from './registry.mjs';
import { countByStatus, renderDispositions, renderRunReport } from './report.mjs';

// `fileURLToPath`, never `.pathname`: a file URL's pathname keeps a leading slash, and on
// Windows the drive letter sits after it, so `readFileSync` opens a doubled-drive path.
const registryPath = fileURLToPath(new URL('../../acceptance/criteria.json', import.meta.url));

test('the committed disposition table matches the registry', () => {
  const registry = loadRegistry(registryPath);
  const onDisk = readFileSync(
    fileURLToPath(new URL('../../acceptance/DISPOSITIONS.md', import.meta.url)),
    'utf8',
  );
  assert.equal(renderDispositions(registry), onDisk);
});

test('the table states criterion 29 as not verified here', () => {
  const rendered = renderDispositions(loadRegistry(registryPath));
  assert.match(rendered, /\| 29 \| external \|/u);
  assert.match(rendered, /beta/iu);
});

test('every check that carries a reason prints it in the table', () => {
  const registry = loadRegistry(registryPath);
  const rendered = renderDispositions(registry);
  for (const entry of registry.criteria) {
    for (const check of entry.checks) {
      if (typeof check.reason !== 'string') continue;
      assert.ok(rendered.includes(check.reason), `${check.id}'s reason is missing from the table`);
    }
  }
});

test('countByStatus counts checks, not criteria', () => {
  const counts = countByStatus(loadRegistry(registryPath));
  assert.ok(counts.automated > 0);
  assert.equal(counts.external, 1);
});

test('the run report names the failing tests rather than counting them', () => {
  const registry = loadRegistry(registryPath);
  const join = joinResults(registry, []);
  const rendered = renderRunReport(
    registry,
    join,
    { newFailures: ['a::b'], stale: [], missing: [] },
    [],
  );
  assert.match(rendered, /a::b/u);
});

test('the run report says which suites did not run rather than implying they passed', () => {
  const registry = loadRegistry(registryPath);
  const join = joinResults(registry, []);
  const rendered = renderRunReport(
    registry,
    join,
    { newFailures: [], stale: [], missing: [] },
    [],
    ['e2e', 'vitest'],
  );
  assert.match(rendered, /Suites not run here: e2e, vitest/u);
  assert.match(rendered, /partial run/u);
});
