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
  // [p3-36] 171 + AC-22-gpu-residency, the one phase-1 check phase 3 adds, + AC-42-register-audit.
  assert.equal(counts[1].checks, 173);
  // [p3] The third phase is a key with a count in it, not an absent key: `byPhase[3] += 1` on an
  // object without a `3` yields NaN, and the run report prints `Phase 3: NaN checks`. [p3-36] The
  // figure is the register's as registered on this branch, measured rather than predicted.
  assert.deepEqual(
    { criteria: counts[3].criteria, checks: counts[3].checks },
    {
      criteria: 146,
      checks: 338,
    },
  );
  // [p2] §20's thirteen plus §21's fifteen plus §23's twelve plus §25's twenty-six. Phase 1's
  // figures are the ones that must not move.
  assert.equal(counts[2].criteria, 104);
  assert.equal(counts[2].checks, 225);
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
    '320 criteria / 736 checks validated — phase 1 70/173, phase 2 104/225, phase 3 146/338',
  );
  assert.equal(
    renderRegistryLine({ criteria: [] }),
    '0 criteria / 0 checks validated — phase 1 0/0, phase 2 0/0, phase 3 0/0',
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

// [p3] The empty-section sentence was a hard-coded phase in a branch that runs for whichever
// phase is empty, so a wave-0 table would have said "No phase-2 criteria are registered yet"
// under the Phase 3 heading, beside a full phase-2 table. Its own comment says a reader must be
// able to tell "none registered yet" from "the renderer stopped reading" — a wrong phase name
// says neither. [p3-36] Turned around: phase 3 is registered, so its heading carries a criterion
// table and the sentence is said of no phase — while an empty register still says it, in its name.
test('the table names the third phase, renders its criteria, and says nothing is missing', () => {
  const rendered = renderDispositions(loadRegistry(registryPath));
  assert.match(rendered, /## Phase 3/u);
  const phase3 = rendered.slice(rendered.indexOf('## Phase 3'));
  assert.match(phase3, /\| P3-\d{2}-\d{1,2}[a-c]? \| /u);
  assert.doesNotMatch(rendered, /no phase-\d criteria are registered yet/iu);
  assert.match(
    renderDispositions({ version: 1, criteria: [] }),
    /No phase-3 criteria are registered yet\./u,
  );
});

test('a phase-3 check is counted, not added to a key that is not there', () => {
  const registry = {
    version: 1,
    criteria: [
      {
        id: 'P3-28-1',
        title: 'A phase-3 criterion',
        group: 'subsystems',
        spec: '§28.1',
        checks: [
          {
            id: 'AC-P3-28-1',
            status: 'automated',
            runner: 'cargo',
            test: 'a::b',
            assert: 'a'.repeat(12),
          },
        ],
      },
    ],
  };
  const join = joinResults(registry, []);
  const rendered = renderRunReport(registry, join, { newFailures: [], stale: [], missing: [] }, []);
  assert.match(rendered, /Phase 3: 1 checks/u);
  assert.ok(!rendered.includes('NaN'), 'a missing phase key prints NaN and reads as a count');
});

test('the run report splits its check count by phase', () => {
  const registry = loadRegistry(registryPath);
  const join = joinResults(registry, []);
  const rendered = renderRunReport(registry, join, { newFailures: [], stale: [], missing: [] }, []);
  assert.match(rendered, /Phase 1: 173 checks/u);
  assert.match(rendered, /Phase 2: 225 checks/u);
  assert.match(rendered, /Phase 3: 338 checks/u);
});
