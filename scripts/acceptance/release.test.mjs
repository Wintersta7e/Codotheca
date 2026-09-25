import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { joinResults } from './join.mjs';
import { loadRegistry } from './registry.mjs';
import {
  RELEASE_NOT_RUN,
  derivedNotRun,
  readmeRegisterFigures,
  releaseProblems,
} from './release.mjs';
import { renderRegistryLine } from './report.mjs';

const registryPath = fileURLToPath(new URL('../../acceptance/criteria.json', import.meta.url));

const check = (over) => ({ runner: 'cargo', assert: 'a'.repeat(12), ...over });
const entry = (id, checks) => ({ id, title: id, group: 'functional', spec: '§16.1', checks });
const record = { recordedAt: '2026-09-25', evidence: 'e'.repeat(30), commit: 'c'.repeat(40) };

/**
 * One of every shape the 1.0 mode meets: four the register exempts, three it refuses and two
 * that are clean. The results and the record status are injected, so the rule is tested without
 * a capture or a git history.
 */
const mixed = {
  version: 1,
  criteria: [
    entry('20', [
      check({
        id: 'AC-20-scan',
        status: 'deferred',
        runner: 'perf',
        owner: '07',
        test: 'perf::scan',
      }),
    ]),
    entry('29', [
      check({ id: 'AC-29-linux-gpu', status: 'external', runner: 'none', reason: 'r'.repeat(30) }),
    ]),
    entry('30', [
      check({ id: 'AC-30-idle', status: 'unmeasurable', runner: 'none', reason: 'r'.repeat(30) }),
    ]),
    entry('26', [
      check({
        id: 'AC-26-roundtrip',
        status: 'deferred',
        runner: 'e2e',
        owner: '19',
        test: 'AC-26 x',
      }),
    ]),
    entry('1', [check({ id: 'AC-1-once', status: 'automated', test: 'a::not_run' })]),
    entry('2', [check({ id: 'AC-2-plan', status: 'deferred', owner: '09', test: 'a::passes' })]),
    entry('13', [
      check({
        id: 'AC-13-focus',
        status: 'manual',
        runner: 'manual',
        gate: 'SECOND-LAUNCH-FOCUS',
        reason: 'r'.repeat(30),
      }),
    ]),
    entry('21', [
      check({
        id: 'AC-21-audio',
        status: 'manual',
        runner: 'manual',
        gate: 'IDLE-AUDIO',
        reason: 'r'.repeat(30),
        record,
      }),
    ]),
    entry('P2-20-13', [
      check({
        id: 'AC-P2-20-13',
        status: 'deferred',
        deferral: 'live-observation',
        runner: 'none',
        owner: 'p2-20',
        reason: 'r'.repeat(30),
        verification: { ...record },
      }),
    ]),
  ],
};
const mixedResults = [{ id: 'a::passes', status: 'passed', runner: 'cargo', tags: ['2'] }];
const recordStatus = (c) =>
  c.record === undefined || c.record === null
    ? { counts: false, why: 'no record' }
    : { counts: true, why: 'recorded' };

/** The README sentence the register's own line would publish, for a clean comparison. */
const readmeFor = (registry) => {
  const [, criteria, checks] = /^(\d+) criteria \/ (\d+) checks/u.exec(
    renderRegistryLine(registry),
  );
  return `An acceptance register tying ${checks} checks to ${criteria} written criteria.`;
};

test('ac_p4_48_27 the 1.0 mode prints the not-run set it derives and fails on any other', () => {
  const joined = joinResults(mixed, mixedResults, recordStatus);
  const { notRun, problems, refused } = releaseProblems(mixed, joined, '1.0', {
    readme: readmeFor(mixed),
    porcelain: '',
  });
  assert.equal(refused, null);
  // The exemption is derived by runner and status, never listed by hand: perf, external,
  // unmeasurable, and the closed list.
  assert.deepEqual(
    notRun.map((n) => n.id),
    ['AC-20-scan', 'AC-29-linux-gpu', 'AC-30-idle', 'AC-26-roundtrip'],
  );
  for (const n of notRun) assert.ok(n.why.length > 0, n.id);
  const named = (id) => problems.some((p) => p.startsWith(`${id}:`));
  assert.ok(named('AC-1-once'), 'an automated check that did not run');
  assert.ok(named('AC-2-plan'), 'a phase-1 plan deferral, even one whose test passes');
  assert.ok(named('AC-13-focus'), 'a manual check with no counting record');
  for (const clean of ['AC-21-audio', 'AC-P2-20-13', ...notRun.map((n) => n.id)]) {
    assert.ok(!named(clean), `${clean} is not a problem`);
  }
  assert.equal(problems.length, 3, problems.join('\n'));
});

test('ac_p4_48_27 RELEASE_NOT_RUN is closed and every entry carries its ruling', () => {
  assert.deepEqual(
    RELEASE_NOT_RUN.map((r) => r.id),
    ['AC-26-roundtrip', 'AC-27-appimage', 'AC-27-packages', 'AC-P2-21-3-floor'],
  );
  const registered = new Set(
    loadRegistry(registryPath).criteria.flatMap((c) => c.checks.map((k) => k.id)),
  );
  for (const { id, ruling } of RELEASE_NOT_RUN) {
    assert.ok(String(ruling).length >= 20, `${id} is exempt with no ruling`);
    assert.ok(registered.has(id), `${id} is not a registered check`);
  }
  // The exemption follows the list and nothing else: a check off the list is not exempted by name.
  assert.deepEqual(
    derivedNotRun({
      criteria: [
        entry('1', [check({ id: 'AC-1-x', status: 'deferred', owner: '07', test: 'a::b' })]),
      ],
    }),
    [],
  );
});

test('ac_p4_48_27 a release run refuses a dirty tree', () => {
  const { refused, problems } = releaseProblems(mixed, joinResults(mixed, []), '1.0', {
    readme: readmeFor(mixed),
    porcelain: ' M README.md\n?? stray.txt\n',
  });
  assert.match(String(refused), /dirty/u);
  assert.match(String(refused), /README\.md/u);
  assert.deepEqual(problems, [], 'a refused run grades nothing');
  for (const mode of ['ft', '1.0']) {
    assert.equal(
      releaseProblems(mixed, joinResults(mixed, []), mode, { readme: '', porcelain: '\n' }).refused,
      null,
      'an empty status is a clean tree',
    );
  }
});

test('ac_p4_48_11 a README quoting no register figure fails rather than passing on nothing', () => {
  const joined = joinResults(mixed, mixedResults, recordStatus);
  for (const mode of ['ft', '1.0']) {
    const { problems } = releaseProblems(mixed, joined, mode, {
      readme: 'A README that states no figure at all.',
      porcelain: '',
    });
    assert.ok(
      problems.some((p) => p.includes('quotes no register figure')),
      `${mode}\n${problems.join('\n')}`,
    );
  }
  assert.deepEqual(readmeRegisterFigures('no figure here'), []);
});

// The README half the two modes share. Untagged: `AC-P4-48-11`'s bare check is the README against
// the real register, and a Lane-0 lane lands it; this is the comparison's own logic.
test('every figure pair the README quotes is read, and each must equal the register', () => {
  assert.deepEqual(
    readmeRegisterFigures(
      'tying 1,317 checks to 573 written criteria. Later: 12 checks over\n3 criteria.',
    ),
    [
      { checks: 1317, criteria: 573 },
      { checks: 12, criteria: 3 },
    ],
  );
  const joined = joinResults(mixed, mixedResults, recordStatus);
  const stale = releaseProblems(mixed, joined, 'ft', {
    readme: 'An acceptance register tying 737 checks to 320 written criteria.',
    porcelain: '',
  });
  assert.ok(stale.problems.some((p) => p.includes('737 checks') && p.includes('320 criteria')));
  const fresh = releaseProblems(mixed, joined, 'ft', { readme: readmeFor(mixed), porcelain: '' });
  assert.ok(!fresh.problems.some((p) => p.includes('README')), fresh.problems.join('\n'));
});

// FT is a bar, not a criterion, so these tests carry no criterion tag.
const ft = {
  version: 1,
  criteria: [
    entry('P4-45-1', [
      check({
        id: 'AC-P4-45-1',
        status: 'automated',
        owner: 'p4-L0a',
        test: 'a::ok',
        firstTag: true,
      }),
      check({ id: 'AC-P4-45-1-later', status: 'deferred', owner: 'p4-46b', test: 'a::later' }),
    ]),
    entry('P4-45-3', [
      check({
        id: 'AC-P4-45-3',
        status: 'manual',
        runner: 'manual',
        owner: 'p4-L0a',
        gate: 'WINDOWS-NATIVE',
        reason: 'r'.repeat(30),
        platforms: ['windows'],
        firstTag: true,
        record,
      }),
    ]),
  ],
};

test('the ft mode passes when every first-tag check passed or is recorded', () => {
  const joined = joinResults(
    ft,
    [{ id: 'a::ok', status: 'passed', runner: 'cargo', tags: [] }],
    recordStatus,
  );
  const { problems } = releaseProblems(ft, joined, 'ft', { readme: readmeFor(ft), porcelain: '' });
  // The deferred check owned by a later lane gates 1.0, not the first tag.
  assert.deepEqual(problems, []);
});

test('the ft mode names every first-tag check that is deferred, failed or unrecorded', () => {
  const broken = structuredClone(ft);
  broken.criteria[0].checks[0].status = 'deferred';
  broken.criteria[1].checks[0].record = null;
  const joined = joinResults(broken, [], recordStatus);
  const { problems } = releaseProblems(broken, joined, 'ft', {
    readme: readmeFor(broken),
    porcelain: '',
  });
  assert.ok(
    problems.some((p) => p.startsWith('AC-P4-45-1:')),
    problems.join('\n'),
  );
  assert.ok(
    problems.some((p) => p.startsWith('AC-P4-45-3:')),
    problems.join('\n'),
  );
  assert.ok(!problems.some((p) => p.startsWith('AC-P4-45-1-later:')));
  const failed = joinResults(
    ft,
    [{ id: 'a::ok', status: 'failed', runner: 'cargo', tags: [] }],
    recordStatus,
  );
  assert.ok(
    releaseProblems(ft, failed, 'ft', { readme: readmeFor(ft), porcelain: '' }).problems.some((p) =>
      p.startsWith('AC-P4-45-1:'),
    ),
  );
});
