import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { joinResults } from './join.mjs';
import { loadRegistry } from './registry.mjs';
import {
  countByPhase,
  countByStatus,
  renderDispositions,
  renderRegistryLine,
  renderRunReport,
} from './report.mjs';

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

test('countByPhase splits the register without a second file', () => {
  const registry = loadRegistry(registryPath);
  const counts = countByPhase(registry);
  assert.equal(counts[1].criteria, 70);
  assert.equal(counts[1].checks, 171);
  // [p2] §20's thirteen. Phase 1's figures are the ones that must not move.
  assert.equal(counts[2].criteria, 13);
  assert.equal(counts[2].checks, 23);
  assert.equal(
    Object.values(counts[1].byStatus).reduce((a, b) => a + b, 0),
    counts[1].checks,
  );
  assert.equal(
    Object.values(counts[2].byStatus).reduce((a, b) => a + b, 0),
    counts[2].checks,
  );
});

test('the line a successful run prints states what it validated, per phase', () => {
  // The whole string, not a fragment: this is the only thing a passing acceptance run says
  // about the register, and a run that validated nothing must not read like a clean one.
  assert.equal(
    renderRegistryLine(loadRegistry(registryPath)),
    '83 criteria / 194 checks validated — phase 1 70/171, phase 2 13/23',
  );
  assert.equal(
    renderRegistryLine({ criteria: [] }),
    '0 criteria / 0 checks validated — phase 1 0/0, phase 2 0/0',
  );
});

test('the table names both phases, and says phase 2 holds nothing yet rather than nothing', () => {
  const rendered = renderDispositions(loadRegistry(registryPath));
  assert.match(rendered, /## Phase 1/u);
  assert.match(rendered, /## Phase 2/u);
  // [p2] It no longer holds nothing: §20 is registered, so the table names its criteria instead
  // of the placeholder. The placeholder must be gone — a table still saying "none yet" over
  // thirteen rows would be the register lying about itself.
  assert.doesNotMatch(rendered, /no phase-2 criteria are registered yet/iu);
  assert.match(rendered, /P2-20-1/u);
});

test('the run report splits its check count by phase', () => {
  const registry = loadRegistry(registryPath);
  const join = joinResults(registry, []);
  const rendered = renderRunReport(registry, join, { newFailures: [], stale: [], missing: [] }, []);
  assert.match(rendered, /Phase 1: 171 checks/u);
  assert.match(rendered, /Phase 2: 23 checks/u);
});
