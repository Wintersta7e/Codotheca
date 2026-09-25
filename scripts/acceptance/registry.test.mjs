import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  DEFERRALS,
  LETTERED_P3,
  LIVE_OBSERVATION_CHECKS,
  PHASE2_SECTIONS,
  PHASE3_SECTIONS,
  criterionOf,
  loadRegistry,
  phaseOf,
  rollUp,
  validatePhase2Complete,
  validatePhase3Complete,
  validateRegistry,
} from './registry.mjs';
import { tagsIn } from './tags.mjs';

// `fileURLToPath`, never `.pathname`: a file URL's pathname keeps a leading slash, and on
// Windows the drive letter sits after it, so `readFileSync` opens a doubled-drive path.
const registryPath = fileURLToPath(new URL('../../acceptance/criteria.json', import.meta.url));
const repoRoot = fileURLToPath(new URL('../..', import.meta.url));

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
  // Phase 1 only. A phase-2 id is `P2-20-1`, which `parseInt` reads as NaN and which would
  // otherwise swell this set by one and hide a genuinely missing phase-1 criterion behind a
  // plausible total.
  const integers = new Set(
    registry.criteria.filter((c) => phaseOf(c.id) === 1).map((c) => Number.parseInt(c.id, 10)),
  );
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
    // The phase-2 lanes. Same rule: a literal rather than a directory listing, because the
    // plans live under a gitignored directory and a test that read them would scan nothing on
    // a fresh clone and pass on nothing.
    'p2-20',
    'p2-21',
    'p2-22',
    'p2-23',
    'p2-24',
    'p2-24b',
    'p2-25',
    'p2-25b',
    'p2-26',
    // The phase-3 lanes, wave 0 through wave 6.
    'p3-28',
    'p3-29',
    'p3-30',
    'p3-31',
    'p3-32',
    'p3-33',
    'p3-34',
    'p3-35',
    'p3-36',
    'p3-36a',
  ]);
  // The phase-3 lanes, in the set before the first `AC-P3-*` id is written anywhere. Same rule
  // again: a literal, because the plans live under a gitignored directory.
  for (const id of [
    'p3-28',
    'p3-29',
    'p3-30',
    'p3-31',
    'p3-32',
    'p3-33',
    'p3-34',
    'p3-35',
    'p3-36',
    'p3-36a',
  ]) {
    assert.ok(plans.has(id), `the plan set does not know ${id}`);
  }
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

// ---------------------------------------------------------------------------------------------
// The second phase. The phase-1 forms above are untouched on purpose: a second id form is added
// beside the first rather than replacing it, so every assertion in this file still reads the
// behaviour phase 1 shipped with.
// ---------------------------------------------------------------------------------------------

// `automated` by default, because from Task 12 onwards a phase-2 deferral to a plan is itself a
// problem: every phase-2 plan has merged, so the fixture's default status has to be the one a
// correct entry carries or every assertion below reads that problem instead of its own.
const p2Check = (over = {}) => ({
  id: 'AC-P2-20-1',
  status: 'automated',
  runner: 'cargo',
  owner: 'p2-20',
  test: 'acceptance_p2_accounts::ac_p2_20_1',
  assert: 'a'.repeat(12),
  ...over,
});

const p2Entry = (over = {}) => ({
  id: 'P2-20-1',
  title: 'A phase-2 criterion',
  group: 'subsystems',
  spec: '§20.15',
  checks: [p2Check()],
  ...over,
});

/** §20's criteria `1..n`, so a section-completeness assertion has a whole section to read. */
function section20(n) {
  const criteria = [];
  for (let i = 1; i <= n; i += 1) {
    criteria.push(
      p2Entry({
        id: `P2-20-${String(i)}`,
        checks: [
          p2Check({
            id: `AC-P2-20-${String(i)}`,
            test: `acceptance_p2_accounts::ac_p2_20_${String(i)}`,
          }),
        ],
      }),
    );
  }
  return criteria;
}

/** Only the problems this register's phase-2 half produced. The phase-1 sweep still runs. */
const phase2Problems = (criteria, root = null) =>
  validateRegistry({ version: 1, criteria }, root).filter((p) => p.includes('P2-'));

const liveCheck = (over = {}) => ({
  id: 'AC-P2-20-13',
  status: 'deferred',
  deferral: 'live-observation',
  runner: 'none',
  owner: 'p2-20',
  reason: 'r'.repeat(30),
  verification: { recordedAt: null, evidence: null },
  assert: 'a'.repeat(12),
  ...over,
});

const withLive = (over) => [p2Entry({ id: 'P2-20-13', checks: [liveCheck(over)] })];

test('criterionOf reads a phase-2 criterion out of a phase-2 check id', () => {
  assert.equal(criterionOf('AC-P2-24-3'), 'P2-24-3');
  assert.equal(criterionOf('AC-P2-25-10-ddl'), 'P2-25-10');
  assert.equal(criterionOf('AC-P2-21-3-floor'), 'P2-21-3');
  // §26 holds no criterion of its own, so the id form refuses one rather than a reviewer.
  assert.equal(criterionOf('AC-P2-26-1'), null);
  // The phase-1 forms are unchanged.
  assert.equal(criterionOf('AC-14'), '14');
  assert.equal(criterionOf('AC-45b-grid-and-hero'), '45b');
});

test('phaseOf reads a criterion id or a check id', () => {
  assert.equal(phaseOf('P2-20-1'), 2);
  assert.equal(phaseOf('AC-P2-24-3'), 2);
  assert.equal(phaseOf('45b'), 1);
  assert.equal(phaseOf('AC-14'), 1);
});

test('the section table is §26.1 and adds up to 104', () => {
  assert.deepEqual(PHASE2_SECTIONS, { 20: 13, 21: 17, 22: 13, 23: 12, 24: 23, 25: 26 });
  assert.equal(
    Object.values(PHASE2_SECTIONS).reduce((a, b) => a + b, 0),
    104,
  );
  assert.deepEqual(DEFERRALS, ['plan', 'live-observation']);
  // R56's two and §36.6's three. One id per subject §36.6 names, each a second check on a
  // criterion whose first check is automated, so nothing is registered as observed and nothing
  // loses its fixture test.
  assert.deepEqual(LIVE_OBSERVATION_CHECKS, [
    'AC-P2-20-13',
    'AC-P2-21-3-floor',
    'AC-P3-32-3-header',
    'AC-P3-32-3-resource',
    'AC-P3-32-16-caps',
  ]);
});

test('a live-observation deferral refuses a test id', () => {
  const problems = validateRegistry({ version: 1, criteria: withLive({ test: 'x::y' }) });
  assert.ok(problems.some((p) => p.includes('AC-P2-20-13') && p.includes('test')));
});

test('a date with no record, and a record with no date, are both refused', () => {
  const dated = withLive({ verification: { recordedAt: '2026-09-04', evidence: 'too short' } });
  assert.ok(
    validateRegistry({ version: 1, criteria: dated }).some((p) => p.includes('evidence')),
    'a date without a record is not a record',
  );
  const undated = withLive({ verification: { recordedAt: null, evidence: 'e'.repeat(30) } });
  assert.ok(
    validateRegistry({ version: 1, criteria: undated }).some((p) => p.includes('recordedAt')),
    'evidence with no date it was recorded on is not an observation',
  );
});

test('live-observation is the two ids it is ruled for and nothing else', () => {
  const criteria = [p2Entry({ checks: [liveCheck({ id: 'AC-P2-20-1' })] })];
  assert.ok(
    validateRegistry({ version: 1, criteria }).some((p) => p.includes('live-observation')),
    'a third live-observation needs a ruling, not a field',
  );
});

test('a deferral value the register does not know, and one on a check that is not deferred', () => {
  const unknown = [p2Entry({ checks: [p2Check({ deferral: 'someday' })] })];
  assert.ok(
    validateRegistry({ version: 1, criteria: unknown }).some((p) => p.includes('deferral')),
  );
  const misplaced = [
    p2Entry({
      checks: [
        p2Check({ status: 'automated', deferral: 'plan', test: 'acceptance_p2_accounts::x' }),
      ],
    }),
  ];
  assert.ok(
    validateRegistry({ version: 1, criteria: misplaced }).some((p) => p.includes('deferral')),
  );
});

test('a phase-2 check carries no performance figure at all', () => {
  for (const over of [
    { runner: 'perf' },
    { measurement: { kind: 'size', machines: ['A'], thermal: 'n/a' } },
    { budget: [{ metric: 'bytes', op: '<', value: 1 }] },
  ]) {
    const criteria = [p2Entry({ checks: [p2Check(over)] })];
    const problems = validateRegistry({ version: 1, criteria });
    // The new rule's own words. `performance` alone is satisfied by the pre-existing
    // `a performance check states a measurement or is marked unmeasurable`, which the budget
    // case triggers too — so this assertion passed with the phase-2 rule deleted.
    assert.ok(
      problems.some((p) => p.includes('AC-P2-20-1') && p.includes('no performance sample exists')),
      `phase 2 records no ${JSON.stringify(over)}`,
    );
  }
});

test('a phase-2 criterion cites its own section, never §16', () => {
  const criteria = [p2Entry({ id: 'P2-25-10', spec: '§16.1', checks: [] })];
  assert.ok(validateRegistry({ version: 1, criteria }).some((p) => p.includes('§25.')));
});

test('external is criterion 29 and is not reachable from phase 2', () => {
  const criteria = [
    p2Entry({ checks: [p2Check({ status: 'external', runner: 'none', reason: 'r'.repeat(30) })] }),
  ];
  // The new rule's own words. `external` alone is satisfied by the pre-existing
  // `external is criterion 29 and nothing else`, which fires for any id that is not `29` — so
  // this assertion passed with the phase-2 rule deleted.
  assert.ok(
    validateRegistry({ version: 1, criteria }).some((p) =>
      p.includes('not reachable from phase 2'),
    ),
  );
});

test('the four statements of the phase-2 section range agree', () => {
  // `2[0-5]` is written four times — CHECK_ID_P2, P2_ID, TAG_P2 and this table's keys — and
  // nothing binds them. A section the table does not name has no count to be complete against,
  // and `validatePhase2Complete` would loop `n <= undefined` and report a silent all-clear.
  for (const section of Object.keys(PHASE2_SECTIONS)) {
    assert.equal(criterionOf(`AC-P2-${section}-1`), `P2-${section}-1`, section);
    assert.deepEqual(tagsIn(`ac_p2_${section}_1_x`), [`P2-${section}-1`], section);
    assert.equal(phaseOf(`P2-${section}-1`), 2, section);
  }
  for (const outside of ['19', '26']) {
    assert.equal(criterionOf(`AC-P2-${outside}-1`), null, outside);
    assert.deepEqual(tagsIn(`ac_p2_${outside}_1_x`), [], outside);
    assert.equal(PHASE2_SECTIONS[outside], undefined, outside);
  }
  // Two sections, each holding one of its many criteria, both still reported. The
  // `expected === undefined` branch itself cannot be reached from here — `P2_ID` refuses any
  // section outside `2[0-5]`, which is exactly the binding this test exists to hold. The branch
  // is defence for the day someone widens one of the four statements and not the others.
  assert.ok(
    validatePhase2Complete({ criteria: [{ id: 'P2-25-1' }, { id: 'P2-20-1' }] }).length > 0,
    'an incomplete section is still reported',
  );
});

test('an empty register is refused, which is why no second zero-check exists downstream', () => {
  const problems = validateRegistry({ version: 1, criteria: [] });
  assert.ok(problems.some((p) => p.includes('registry.criteria is empty')));
});

test('a section holding twelve of its thirteen criteria names the one that is missing', () => {
  const problems = phase2Problems(section20(12));
  assert.deepEqual(problems.length, 1, problems.join('\n'));
  assert.ok(problems[0].includes('P2-20-13'), problems[0]);
});

test('a fully registered section validates, and a section with nothing in it is silent', () => {
  // §21 through §25 hold nothing here. A section with no entry is not yet registered, which is
  // what lets the harness widening land before the first phase-2 criterion exists.
  assert.deepEqual(phase2Problems(section20(13)), []);
});

test('a duplicate inside a section is a problem, not a silent overwrite', () => {
  const criteria = section20(13);
  criteria[12] = { ...criteria[12], id: 'P2-20-12' };
  assert.ok(phase2Problems(criteria).some((p) => p.includes('P2-20')));
});

test('a scanning check says out loud that it prints a count and fails at zero', () => {
  const quiet = [p2Entry({ checks: [p2Check({ scanning: true })] })];
  assert.ok(validateRegistry({ version: 1, criteria: quiet }).some((p) => p.includes('scanning')));
  const loud = [
    p2Entry({
      checks: [
        p2Check({
          scanning: true,
          assert: 'walks core/src, prints the file count and fails at zero files scanned',
        }),
      ],
    }),
  ];
  assert.ok(!validateRegistry({ version: 1, criteria: loud }).some((p) => p.includes('scanning')));
});

test('a source is a citation, a probe or the schema — never a restatement of the number', () => {
  for (const source of ['§8.0a', '§25.3', 'schema', 'probe:shelf-fps']) {
    const criteria = [p2Entry({ checks: [p2Check({ source })] })];
    assert.ok(
      !validateRegistry({ version: 1, criteria }).some((p) => p.includes('source')),
      `${source} is a source`,
    );
  }
  for (const source of ['508px', '100', '', 'the spec']) {
    const criteria = [p2Entry({ checks: [p2Check({ source })] })];
    assert.ok(
      validateRegistry({ version: 1, criteria }).some((p) => p.includes('source')),
      `${JSON.stringify(source)} restates a figure instead of sourcing it`,
    );
  }
});

test('a mirror names a file that exists, and only when the validator is given a root', () => {
  const good = [p2Entry({ checks: [p2Check({ mirror: { other: 'core/src/index/migrate.rs' } })] })];
  assert.ok(
    !validateRegistry({ version: 1, criteria: good }, repoRoot).some((p) => p.includes('mirror')),
  );
  const bad = [p2Entry({ checks: [p2Check({ mirror: { other: 'core/src/nowhere.rs' } })] })];
  assert.ok(
    validateRegistry({ version: 1, criteria: bad }, repoRoot).some((p) => p.includes('mirror')),
  );
  // Without a root the validator does no I/O and checks the shape alone.
  assert.ok(!validateRegistry({ version: 1, criteria: bad }).some((p) => p.includes('mirror')));
  const shapeless = [p2Entry({ checks: [p2Check({ mirror: { other: '' } })] })];
  assert.ok(
    validateRegistry({ version: 1, criteria: shapeless }).some((p) => p.includes('mirror')),
  );
});

test('the freeze holds the nine phase-1 entries the phase-2 sections govern', () => {
  // Not an equality with the live register: §26.3 says those seven criteria *move*, each with
  // the plan that lands its section's body, and an equality here would redden the lane doing
  // the moving. The supersession audits diff against this file; this test proves the record
  // exists and is well-formed, which is the half that has to be right on the day it is written.
  const frozen = JSON.parse(
    readFileSync(fileURLToPath(new URL('../../acceptance/phase1-frozen.json', import.meta.url))),
  );
  assert.equal(frozen.version, 1);
  assert.deepEqual(
    frozen.criteria.map((c) => c.id),
    ['2', '10', '42', '44', '45c', '58', '64'],
  );
  assert.deepEqual(
    frozen.checks.map((c) => c.check.id),
    ['AC-13-no-second-core', 'AC-23-audit'],
  );
  assert.deepEqual(
    frozen.checks.map((c) => c.criterion),
    ['13', '23'],
  );
  // Every frozen entry is a whole entry, not a stub: the record has to be usable as a diff.
  for (const entry of frozen.criteria) {
    assert.ok(entry.checks.length > 0, `${entry.id} was frozen without its checks`);
    assert.ok(typeof entry.title === 'string' && entry.title.length > 0, entry.id);
  }
});

// [p2] This began as wave 0's proof that widening the *validator* added no criterion. §20's
// registration is the first change that legitimately moves the totals, so the assertion moves
// with it — and keeps its real content, which is that **phase 1 is untouched**. A phase-2 lane
// that disturbed a phase-1 criterion still fails here.
// [p3-36] Phase 1 moves twice, deliberately: criterion 22 gains AC-22-gpu-residency (R127.6), the
// one phase-1 check phase 3 adds, and criterion 42 gains AC-42-register-audit, the §26.3 audit
// the node:test capture made visible, registered as an audit of the entry. Any other movement
// still fails here.
test('registering later phases leaves phase 1 at 70 and 173', () => {
  const registry = loadRegistry(registryPath);
  assert.deepEqual(validateRegistry(registry, repoRoot), []);
  const phase1 = registry.criteria.filter((c) => phaseOf(c.id) === 1);
  assert.equal(phase1.length, 70);
  assert.equal(
    phase1.reduce((n, c) => n + c.checks.length, 0),
    173,
  );
  // §20 owns thirteen, §21 seventeen, §22 thirteen, §23 twelve, §24 twenty-three and §25
  // twenty-six, which is what
  // `PHASE2_SECTIONS` declares for each. Raised by this lane's own delta, read from the branch
  // base — never to a running total a later lane would have to guess at.
  assert.equal(registry.criteria.filter((c) => phaseOf(c.id) === 2).length, 104);
});

// [p2-26 Task 4] §22's thirteen. The section is complete or it is silent, and this is what
// turns the silence off.
test('§22 holds exactly thirteen contiguous criteria', () => {
  const registry = loadRegistry(registryPath);
  const ids = registry.criteria
    .filter((c) => /^P2-22-/u.test(c.id))
    .map((c) => Number.parseInt(c.id.slice('P2-22-'.length), 10))
    .sort((a, b) => a - b);
  assert.deepEqual(ids, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
  assert.deepEqual(validatePhase2Complete(registry), []);
});

// Every §22 check names a test that exists in the tree. The `test` field is the join key and
// there is no name table: an id one character off reads as *not run*, and `gateProblems` says so
// against a capture rather than here — which is too late to be a review.
test('every §22 check names a cargo test that the tree really declares', () => {
  const registry = loadRegistry(registryPath);
  const core = fileURLToPath(new URL('../../core/tests/', import.meta.url));
  const checks = registry.criteria
    .filter((c) => /^P2-22-/u.test(c.id))
    .flatMap((c) => c.checks.map((k) => ({ criterion: c.id, ...k })));
  assert.ok(checks.length >= 13, 'the §22 scan found no checks');

  for (const check of checks) {
    assert.equal(check.owner, 'p2-22', `${check.id} is p2-22's`);
    assert.equal(check.status, 'automated', `${check.id} names a test that ran`);
    const [binary, fn] = String(check.test).split('::');
    const source = readFileSync(`${core}${binary}.rs`, 'utf8');
    assert.ok(
      new RegExp(`^(async )?fn ${fn}\\(`, 'mu').test(source),
      `${check.id}: core/tests/${binary}.rs declares no fn ${fn}`,
    );
  }
});

// [p2-26 Tasks 6 and 7] §24 closes at twenty-three, across two owning plans. Half a section
// registered is the state the completeness rule exists to refuse, so this is the assertion that
// turns §24's silence off only once both halves are in.
test('§24 holds exactly twenty-three contiguous criteria, from both its owners', () => {
  const registry = loadRegistry(registryPath);
  const section = registry.criteria.filter((c) => /^P2-24-/u.test(c.id));
  assert.deepEqual(
    section.map((c) => Number.parseInt(c.id.slice('P2-24-'.length), 10)).sort((a, b) => a - b),
    Array.from({ length: 23 }, (_, i) => i + 1),
  );
  const owners = new Set(section.flatMap((c) => c.checks.map((k) => k.owner)));
  assert.deepEqual([...owners].sort(), ['p2-24', 'p2-24b']);
  assert.deepEqual(validatePhase2Complete(registry), []);
});

// R51's fall-through. The criterion has two subjects and two owners, and the second could not be
// written until `RefState.stash_count` became an Option — so this entry was registered against a
// suite and a name that did not exist yet. The merged plan wins: it names what p2-24b landed.
test('P2-25-11 carries two checks, two owners, and no deferral to a plan that has merged', () => {
  const registry = loadRegistry(registryPath);
  const entry = registry.criteria.find((c) => c.id === 'P2-25-11');
  assert.ok(entry, 'P2-25-11 is registered');
  assert.deepEqual(
    entry.checks.map((c) => c.owner),
    ['p2-25', 'p2-24b'],
  );
  for (const check of entry.checks) {
    assert.equal(check.status, 'automated', `${check.id} names a test that ran`);
    assert.equal(check.deferral, undefined, `${check.id} defers to nobody`);
  }
});

// p2-26 Task 1 added the escape so a lane could land a static rule before this plan wrote its
// check. A survivor is a rule with two answers: an escape saying nothing claims it, and a check
// that does — so the rule is discharged by its named criterion being **in the register**, not by
// the rule carrying the field. [p3] Resolved against `criteria.json` rather than asserted as an
// empty list: from wave 1 every phase-3 lane carries an escape for five waves, against a register
// it is not allowed to edit.
test('no static rule carries an escape its criterion has already discharged', () => {
  const read = (name) =>
    JSON.parse(readFileSync(fileURLToPath(new URL(`../../acceptance/${name}`, import.meta.url))));
  const files = { callsites: read('callsites.json'), forbidden: read('forbidden.json') };
  for (const [name, file] of Object.entries(files)) {
    assert.ok((file.rules ?? []).length > 0, `${name} lists no rules — the scan read nothing`);
  }
  assert.deepEqual(
    validatePhase2Complete(loadRegistry(registryPath), files).filter((p) => p.includes('escape')),
    [],
  );
});

// [p2-26 Task 12] R46, mechanically. Each rule below is a way a phase-2 criterion could ship as
// an intention while the gate reported a clean run over it — and each is proved to bite on a
// synthetic case, because an audit nobody proved is an audit that asserts its own defaults.
const p2Complete = (criteria, rules = null) =>
  validatePhase2Complete({ version: 1, criteria }, rules);

test('a phase-2 deferral to a plan is a deferral to nobody', () => {
  const deferred = [
    p2Entry({
      checks: [{ ...p2Check(), status: 'deferred', deferral: 'plan', test: 'a::b' }],
    }),
  ];
  assert.ok(
    p2Complete(deferred).some((p) => p.includes('deferral to one is a deferral to nobody')),
  );
  assert.deepEqual(
    p2Complete([p2Entry()]).filter((p) => p.includes('deferral')),
    [],
  );
});

test('a static rule may not keep an escape a registered check discharges', () => {
  // The named check **is** in this register, which is what discharges the escape. A rule naming
  // a check id resolves, and so does one naming a criterion id: both forms are live in the tree.
  const named = (criterion) => ({
    callsites: { rules: [{ id: 'x', pendingRegistryEntry: { criterion } }] },
    forbidden: { rules: [{ id: 'y' }] },
  });
  for (const criterion of ['AC-P2-20-1', 'P2-20-1']) {
    const problems = p2Complete([p2Entry()], named(criterion));
    assert.ok(
      problems.some((p) => p.includes('callsites:x') && p.includes('escape')),
      criterion,
    );
    assert.ok(!problems.some((p) => p.includes('forbidden:y')));
  }
  // A rule file that lists nothing is the escape scan reading nothing, not a clean tree.
  assert.ok(
    p2Complete([p2Entry()], { callsites: { rules: [] }, forbidden: { rules: [{ id: 'y' }] } }).some(
      (p) => p.includes('the escape scan read nothing'),
    ),
  );
});

// The ordering defect wave 0 exists to close, beside the id validator. p3-36 owns
// `acceptance/criteria.json` and runs last, so every phase-3 lane that lands a static rule before
// its criterion exists would otherwise redden `npm run acceptance` on a gate working correctly
// against a register it may not edit.
test('a rule may wait for a criterion that is not written yet, and not one that is', () => {
  const waiting = {
    callsites: { rules: [{ id: 'name-ban', pendingRegistryEntry: { criterion: 'P3-32-13' } }] },
    forbidden: { rules: [{ id: 'y' }] },
  };
  assert.deepEqual(
    p2Complete([p2Entry()], waiting).filter((p) => p.includes('escape')),
    [],
    'no phase-3 entry exists yet, so nothing has discharged it',
  );

  const registered = [
    p2Entry(),
    p3Entry({
      id: 'P3-32-13',
      spec: '§32.13',
      checks: [p3Check({ id: 'AC-P3-32-13', test: 'acceptance_p3::ac_p3_32_13' })],
    }),
  ];
  assert.ok(
    p2Complete(registered, waiting).some(
      (p) => p.includes('callsites:name-ban') && p.includes('escape'),
    ),
    'once the criterion is registered the rule has two answers to one question',
  );
});

test('two checks may share a join key only when exactly one of them owns it', () => {
  const shared = (over = {}) => [
    p2Entry({ id: 'P2-20-1', checks: [p2Check({ id: 'AC-P2-20-1', test: 'a::b' })] }),
    p2Entry({ id: 'P2-20-2', checks: [p2Check({ id: 'AC-P2-20-2', test: 'a::b', ...over })] }),
  ];
  // Undeclared: a copy-pasted key makes two criteria read as covered by one run.
  assert.ok(p2Complete(shared()).some((p) => p.includes('2 declarers')));
  // Declared, naming a check in its own group: correct, and the shape a two-owner split takes.
  assert.deepEqual(
    p2Complete(shared({ shares: 'AC-P2-20-1' })).filter((p) => p.includes('share')),
    [],
  );
  // Declared at something outside the group, which claims a relationship that is not there.
  assert.ok(
    p2Complete(shared({ shares: 'AC-99-nowhere' })).some((p) => p.includes('not in its group')),
  );
});

test('a phase-2 register with no scanning check and no mirror has stopped reading', () => {
  const bare = [p2Entry({ checks: [p2Check()] })];
  const problems = p2Complete(bare);
  assert.ok(problems.some((p) => p.includes('no scanning check at all')));
  assert.ok(problems.some((p) => p.includes('no mirror at all')));
  // And a criterion with no check at all, which is an id and an intention.
  assert.ok(p2Complete([p2Entry({ checks: [] })]).some((p) => p.includes('carries no check')));
});

// ---------------------------------------------------------------------------------------------
// The third phase. Same rule as the second: the phase-1 and phase-2 forms above are untouched,
// and a third id form is added beside them rather than replacing either.
// ---------------------------------------------------------------------------------------------

test('criterionOf reads a phase-3 criterion out of a phase-3 check id', () => {
  assert.equal(criterionOf('AC-P3-28-1'), 'P3-28-1');
  // Both suffixed ids, never one: a test asserting only `30-11a` passes while `28-18a` is
  // rejected, which is exactly how this widening has failed twice before.
  assert.equal(criterionOf('AC-P3-28-18a'), 'P3-28-18a');
  assert.equal(criterionOf('AC-P3-30-11a'), 'P3-30-11a');
  assert.equal(criterionOf('AC-P3-32-16-caps'), 'P3-32-16');
  // §36 is the registry contract and owns no criterion of its own, and §27 is the scope section.
  assert.equal(criterionOf('AC-P3-36-1'), null);
  assert.equal(criterionOf('AC-P3-27-1'), null);
  // The earlier phases are unchanged.
  assert.equal(criterionOf('AC-14'), '14');
  assert.equal(criterionOf('AC-P2-24-3'), 'P2-24-3');
});

test('PHASE3_SECTIONS is the contiguous range and the two suffixes make up 146', () => {
  // R139 as amended. The constant is the range the contiguity loop walks, never the section
  // total: `PHASE3_SECTIONS[28] = 19` would demand a `P3-28-19` that does not exist, and the
  // register could never validate. §36.1's per-section totals are 19 · 29 · 19 · 18 · 23 · 12 ·
  // 17 · 9 = 146, and `LETTERED_P3`'s two ids are the difference.
  assert.deepEqual(PHASE3_SECTIONS, {
    28: 18,
    29: 29,
    30: 18,
    31: 18,
    32: 23,
    33: 12,
    34: 17,
    35: 9,
  });
  assert.equal(
    Object.values(PHASE3_SECTIONS).reduce((a, b) => a + b, 0),
    144,
  );
  assert.deepEqual(LETTERED_P3, ['P3-28-18a', 'P3-30-11a']);
  assert.equal(Object.values(PHASE3_SECTIONS).reduce((a, b) => a + b, 0) + LETTERED_P3.length, 146);
});

const p3Check = (over = {}) => ({
  id: 'AC-P3-28-1',
  status: 'automated',
  runner: 'cargo',
  owner: 'p3-28',
  test: 'acceptance_p3_debt::ac_p3_28_1',
  assert: 'a'.repeat(12),
  ...over,
});

const p3Entry = (over = {}) => ({
  id: 'P3-28-1',
  title: 'A phase-3 criterion',
  group: 'subsystems',
  spec: '§28.1',
  checks: [p3Check()],
  ...over,
});

test('phaseOf reads the phase out of the id, and phase 1 is no longer the fallback', () => {
  assert.equal(phaseOf('P3-28-1'), 3);
  assert.equal(phaseOf('AC-P3-30-11a'), 3);
  assert.equal(phaseOf('P2-20-1'), 2);
  assert.equal(phaseOf('AC-P2-24-3'), 2);
  assert.equal(phaseOf('45b'), 1);
  assert.equal(phaseOf('AC-14'), 1);
});

test('an id naming a phase the register does not hold says so, and says nothing else', () => {
  // `P4-28-1` was reported as `criterion id outside 1..67` **and** `spec must cite §16.` — two
  // problems that name the wrong defect and read as malformed phase-1 data. A validator that
  // reports the wrong defect is how the last two widenings were mistaken for bad input.
  const problems = validateRegistry({
    version: 1,
    criteria: [p3Entry({ id: 'P4-28-1', checks: [] })],
  });
  assert.ok(
    problems.some((p) => p.includes('names phase 4')),
    problems.join('\n'),
  );
  assert.ok(!problems.some((p) => p.includes('1..67')), 'it is not a malformed phase-1 criterion');
  assert.ok(!problems.some((p) => p.includes('§16.')), 'it cites no section this register knows');
});

test('a phase-3 criterion cites its own section, never §16', () => {
  const criteria = [p3Entry({ id: 'P3-30-11a', spec: '§16.1', checks: [] })];
  assert.ok(validateRegistry({ version: 1, criteria }).some((p) => p.includes('§30.')));
});

test('a phase-3 letter is one of the two ruled ones and nothing else', () => {
  const stray = [p3Entry({ id: 'P3-31-4b', checks: [] })];
  assert.ok(
    validateRegistry({ version: 1, criteria: stray }).some(
      (p) => p.includes('P3-31-4b') && p.includes('letter'),
    ),
  );
  // The id form accepts both ruled ids. A lettered id registered without its bare twin is a
  // different problem, and `validatePhase3Complete` owns it.
  for (const id of ['P3-28-18a', 'P3-30-11a']) {
    const criteria = [p3Entry({ id, checks: [] })];
    assert.ok(
      !validateRegistry({ version: 1, criteria }).some((p) => p.includes('carry a letter')),
      `${id} is a ruled lettered id`,
    );
  }
});

test('a deferred check may name a phase-3 plan, and a phase nobody has is refused', () => {
  const owned = (owner) => [
    p3Entry({ checks: [p3Check({ status: 'deferred', owner, test: 'a::b' })] }),
  ];
  // The earlier forms stay valid: a plan number, a second half, a phase-2 id.
  for (const owner of ['13', '13c', 'p2-20', 'p3-31', 'p3-36a']) {
    assert.ok(
      !validateRegistry({ version: 1, criteria: owned(owner) }).some((p) => p.includes('owner')),
      `${owner} is a plan id`,
    );
  }
  assert.ok(
    validateRegistry({ version: 1, criteria: owned('p4-31') }).some((p) => p.includes('owner')),
  );
});

const p3Live = (over = {}) => [
  p3Entry({
    id: 'P3-32-3',
    spec: '§32.4',
    checks: [
      {
        id: 'AC-P3-32-3-header',
        status: 'deferred',
        deferral: 'live-observation',
        runner: 'none',
        owner: 'p3-32',
        reason: 'r'.repeat(30),
        verification: { recordedAt: null, evidence: null },
        assert: 'a'.repeat(12),
        ...over,
      },
    ],
  }),
];

test('§36.6 names three more live observations, and a fourth still needs a ruling', () => {
  assert.ok(
    !validateRegistry({ version: 1, criteria: p3Live() }).some((p) =>
      p.includes('live-observation'),
    ),
    'AC-P3-32-3-header is ruled',
  );
  const unruled = p3Live({ id: 'AC-P3-32-4' });
  assert.ok(
    validateRegistry({ version: 1, criteria: unruled }).some((p) =>
      p.includes('a third one needs a ruling, not a field'),
    ),
  );
  // The allowlist's design intent is unchanged: it is not relaxed to a pattern, and the rest of
  // the live-observation shape still binds.
  assert.ok(
    validateRegistry({ version: 1, criteria: p3Live({ test: 'x::y' }) }).some(
      (p) => p.includes('AC-P3-32-3-header') && p.includes('test'),
    ),
    'a test standing in for an observation is the assertion this status refuses',
  );
  assert.ok(
    validateRegistry({
      version: 1,
      criteria: p3Live({ verification: { recordedAt: '2026-09-18', evidence: 'too short' } }),
    }).some((p) => p.includes('evidence')),
    'a date is not a record',
  );
});

/** A whole phase-3 section `1..n`, so a completeness assertion has a section to read. */
function p3Section(section, n) {
  const criteria = [];
  for (let i = 1; i <= n; i += 1) {
    criteria.push(
      p3Entry({
        id: `P3-${section}-${String(i)}`,
        spec: `§${section}.1`,
        checks: [
          p3Check({
            id: `AC-P3-${section}-${String(i)}`,
            test: `acceptance_p3::ac_p3_${section}_${String(i)}`,
          }),
        ],
      }),
    );
  }
  return criteria;
}

const p3Complete = (criteria) => validatePhase3Complete({ version: 1, criteria });

test('a §28 holding seventeen of its eighteen names the one that is missing', () => {
  const problems = p3Complete(p3Section(28, 17)).filter((p) => p.includes('P3-28-18'));
  assert.equal(problems.length, 1, problems.join('\n'));
  assert.ok(problems[0].includes('holds 17 of its 18'), problems[0]);
});

test('a duplicate inside a phase-3 section is a problem, not a silent overwrite', () => {
  const criteria = p3Section(29, 29);
  criteria[4] = { ...criteria[4], id: 'P3-29-4' };
  assert.ok(p3Complete(criteria).some((p) => p.includes('P3-29-4 appears 2 times')));
});

test('a lettered id never reaches the contiguity list, so its bare twin is not a duplicate', () => {
  // Measured, not assumed: this is why a lettered id folded into `ns` makes its bare twin report
  // a duplicate that exists in no register — on the one section that now holds two of them.
  assert.equal(Number.parseInt('18a', 10), 18);
  const criteria = [
    ...p3Section(28, 18),
    p3Entry({
      id: 'P3-28-18a',
      spec: '§28.2',
      checks: [p3Check({ id: 'AC-P3-28-18a', test: 'acceptance_p3::ac_p3_28_18a' })],
    }),
  ];
  assert.deepEqual(
    p3Complete(criteria).filter((p) => p.includes('appears')),
    [],
  );
});

test('a phase-3 criterion outside its section range says which range', () => {
  const criteria = [
    p3Entry({
      id: 'P3-33-13',
      spec: '§33.1',
      checks: [p3Check({ id: 'AC-P3-33-13', test: 'acceptance_p3::ac_p3_33_13' })],
    }),
  ];
  assert.ok(p3Complete(criteria).some((p) => p.includes("P3-33-13 is outside §33's 1..12")));
});

test('a lettered id is an additional id beside the bare one, never instead of it', () => {
  const lettered = [
    p3Entry({
      id: 'P3-30-11a',
      spec: '§30.6',
      checks: [p3Check({ id: 'AC-P3-30-11a', test: 'acceptance_p3::ac_p3_30_11a' })],
    }),
  ];
  assert.ok(p3Complete(lettered).some((p) => p.includes('P3-30-11a') && p.includes('P3-30-11')));
  // And the other direction: §30 registered without its suffixed id is half a ruling.
  assert.ok(p3Complete(p3Section(30, 18)).some((p) => p.includes('P3-30-11a')));
});

test('a phase-3 check carries no performance figure and is never external', () => {
  for (const over of [
    { runner: 'perf' },
    { measurement: { kind: 'size', machines: ['A'], thermal: 'n/a' } },
    { budget: [{ metric: 'bytes', op: '<', value: 1 }] },
  ]) {
    assert.ok(
      p3Complete([p3Entry({ checks: [p3Check(over)] })]).some((p) =>
        p.includes('phase 3 adds none'),
      ),
      `phase 3 records no ${JSON.stringify(over)}`,
    );
  }
  const external = [p3Entry({ checks: [p3Check({ status: 'external', runner: 'none' })] })];
  assert.ok(p3Complete(external).some((p) => p.includes('not reachable from phase 3')));
});

test('a phase-3 deferral to a plan is a deferral to nobody, and a bare register has stopped reading', () => {
  const deferred = [
    p3Entry({ checks: [p3Check({ status: 'deferred', deferral: 'plan', test: 'a::b' })] }),
  ];
  assert.ok(
    p3Complete(deferred).some((p) => p.includes('deferral to one is a deferral to nobody')),
  );
  const bare = p3Complete([p3Entry()]);
  assert.ok(bare.some((p) => p.includes('no scanning check at all')));
  assert.ok(bare.some((p) => p.includes('no mirror at all')));
  assert.ok(p3Complete([p3Entry({ checks: [] })]).some((p) => p.includes('carries no check')));
  // An empty phase-3 register is silent, which is what let this land in wave 0. [p3-36] Asserted
  // over an empty register rather than the shipped one, which is no longer empty: the shipped
  // register's completeness is the section tests' and `validateRegistry`'s.
  assert.deepEqual(validatePhase3Complete({ version: 1, criteria: [] }), []);
});

// [p3] The record of what §36.3 moves, taken before wave 1 can touch it. `phase1-frozen.json` is
// **not** extended: it is p2-26's record of a different set at a different tree, and editing it
// destroys the phase-2 audit's own baseline.
//
// `AC-50-zero-animation` is the row that proves the freeze is needed: its assert names ten class
// names today, it named nine before that against a real ten, and R112 requires the literal be
// deleted rather than incremented. Without a frozen copy p3-36's audit cannot tell *deleted per
// the ruling* from *never touched*.
/**
 * [p3] The criteria §36.3 says have moved, and **which plan landed each**.
 *
 * §36.3's own rule is *"each row is applied by the plan that lands its section's body, never ahead
 * of it"*, so the deep-equal below stops being true one row at a time as the waves land. A bare
 * deep-equal over all fourteen therefore blocks every §36.3 row from landing at all, and relaxing
 * it wholesale would throw away the freeze.
 *
 * The narrowing is `check-destructive-tokens.mjs`'s site shape: **one id, one owner, one written
 * reason**. A row that moves without an entry here still fails, which is what keeps this a record
 * of what was decided rather than a hole. `acceptance/phase3-frozen.json` itself is **never
 * edited** — it is p3-36's audit input, and the audit's question is exactly *which of these
 * fourteen moved*.
 */
const MOVED_BY_PLAN = [
  {
    id: '21',
    plan: 'p3-34',
    why: '§36.3 row 21: the surge sits beside the specular sweep, and the sentence bans a schedule, not a response. AC-21-frames gains the clause §16 row 21 now carries: both are bounded one-shots for a change the user is present for, and idle requires no project page open, so neither can breach the budget. Applied by p3-36 for §34, whose lane merged without it.',
  },
  {
    id: '22',
    plan: 'p3-36',
    why: "§36.3 / R127.6: criterion 22's GPU clause gets one value and one owner. AC-22-gpu-residency states the cap at ~165 MB — §7.6's re-derivation against the real card, superseding the < 128 MB that re-derivation moved away from — as unmeasurable, with no budget, because no residency instrument exists in either language. AC-22-art-cache's 'the GPU texture clause is phase 3 and is not tested' now points at that check. D9's gate stays on AC-30-pacing's budget.",
  },
  {
    id: '46',
    plan: 'p3-33',
    why: '§36.3 / A1: the exemption loses *material layers*. Under A1 the five layers are composed in the DOM, so their colours reach decay.css and every one must resolve to a declared token — --dust, --silver, --rust, --fail, --growth. Vents and plate stops stay exempt, because a vent really is drawn into the bitmap. Left as it was, five colour literals would have shipped past a green run, pre-exempted by name.',
  },
  {
    id: '50',
    plan: 'p3-33',
    why: "§36.3 / §36.2 rule 9: the hand-maintained class-name count is REMOVED, not incremented. It read nine against a real ten, c256eca corrected it to ten, and §33 and §34 each add one more without being able to see the other — three moves in one round. The checker now derives the set from motion.css, prints it, and fails at zero. §34's `.cdt-surge` half lands in wave 5 against the same derived checker, so this entry covers the joint row.",
  },
  {
    id: '58',
    plan: 'p3-33',
    why: '§36.3: three dots on an open project page, not two. §33.7 draws the two-clock dial — the interaction clock it already had, plus conditionMaterial as the inner needle, which phase 1 computed and stored and no wire type carried until R138 added the field.',
  },
  {
    id: '62',
    plan: 'p3-33',
    why: "§36.3: the clause said decay rendering is not in phase 1 and is not tested. §33.11's twelve AC-P3-33-* ids now test it, so that half expires. The first-commit-sha half of the same sentence is untouched and stays — it is a seed assertion §33 has no quarrel with, and deleting it along with the decay half would retire a criterion nobody moved. [p3-36] AC-62-fade's phase pin moved too, on the owing lane's behalf: fade's two values are an invariant (§33.2), not a phase-1 pin.",
  },
  {
    id: '66',
    plan: 'p3-35',
    why: "§36.3: AC-66-sort gains `needs_attention` and `Completion` stays absent. The check also flips deferred → automated, because §35.6's AC-P3-35-4 is the first test to implement it — it derives the cycle's membership from protocol/schema/protocol.json rather than counting it by hand.",
  },
];

test('the freeze holds the fourteen entries the phase-3 sections govern', () => {
  const frozen = JSON.parse(
    readFileSync(fileURLToPath(new URL('../../acceptance/phase3-frozen.json', import.meta.url))),
  );
  assert.equal(frozen.version, 1);
  assert.deepEqual(
    frozen.criteria.map((c) => c.id),
    ['21', '22', '45a', '45b', '45c', '46', '50', '58', '62', '64', '65', '66', '67', 'P2-25-5'],
  );
  assert.equal(frozen.criteria.length, 14);

  // Every entry is registered against one of the frozen ids, with an owner and a written reason.
  // An entry for a row that has not moved is as much a defect as a move with no entry.
  const moved = new Map(MOVED_BY_PLAN.map((row) => [row.id, row]));
  assert.equal(moved.size, MOVED_BY_PLAN.length, 'one entry per moved criterion');
  for (const row of MOVED_BY_PLAN) {
    assert.ok(
      frozen.criteria.some((c) => c.id === row.id),
      `${row.id} is not one of the frozen fourteen`,
    );
    assert.match(String(row.plan), /^p3-\d{2}[a-c]?$/u, `${row.id} needs an owning plan`);
    assert.ok(String(row.why).length > 40, `${row.id} moved with no written reason`);
  }

  // Deep-equal to the live entry for every row that has **not** been registered as moved. This
  // is what p3-36's audit reads: a row that differs and is listed above moved on purpose, and a
  // row that differs and is not listed fails right here.
  const live = new Map(loadRegistry(registryPath).criteria.map((c) => [String(c.id), c]));
  let held = 0;
  for (const entry of frozen.criteria) {
    assert.ok(live.has(entry.id), `${entry.id} is not in criteria.json`);
    assert.ok(entry.checks.length > 0, `${entry.id} was frozen without its checks`);
    if (moved.has(entry.id)) {
      assert.notDeepEqual(
        entry,
        live.get(entry.id),
        `${entry.id} is registered as moved by ${moved.get(entry.id).plan} and has not moved`,
      );
      continue;
    }
    assert.deepEqual(entry, live.get(entry.id), `${entry.id} has already moved`);
    held += 1;
  }
  assert.equal(held, frozen.criteria.length - MOVED_BY_PLAN.length);
});

// [p4] The record of what §49.3 moves, taken from the phase-4 base rather than the working tree, so
// it is right whichever lane merges first. `phase3-frozen.json` is not extended: it is a record of a
// different set at a different tree. Fifty-five entries: §49.3's fifty-two rows, R157's `P3-28-5`
// and `-15`, and R222's `P3-34-11`.
const PHASE4_FROZEN_IDS = [
  '3',
  '8',
  '9',
  '14',
  '21',
  '25',
  '26',
  '27',
  '28',
  '29',
  '44',
  '50',
  '51',
  '54',
  '64',
  '66',
  'P2-20-2',
  'P2-20-13',
  'P2-21-3',
  'P2-24-1',
  'P2-24-2',
  'P2-24-3',
  'P2-24-4',
  'P2-24-5',
  'P2-24-6',
  'P2-24-13',
  'P2-24-14',
  'P2-24-15',
  'P2-24-16',
  'P2-24-18',
  'P2-24-21',
  'P2-25-11',
  'P3-28-5',
  'P3-28-14',
  'P3-28-15',
  'P3-30-11',
  'P3-30-13',
  'P3-31-11',
  'P3-31-14',
  'P3-32-3',
  'P3-32-12',
  'P3-32-13',
  'P3-32-14',
  'P3-32-16',
  'P3-33-8',
  'P3-34-11',
  'P3-34-14',
  'P3-34-15',
  'P3-35-1',
  'P3-35-3',
  'P3-35-4',
  'P3-35-5',
  'P3-35-6',
  'P3-35-7',
  'P3-35-9',
];

const phase4Frozen = () =>
  JSON.parse(
    readFileSync(
      fileURLToPath(new URL('../../acceptance/phase4-frozen.json', import.meta.url)),
      'utf8',
    ),
  );

test('phase4-frozen.json holds §49.3’s entries and every one is registered', () => {
  const frozen = phase4Frozen();
  assert.equal(frozen.version, 1);
  assert.equal(frozen.frozenFrom, 'acceptance/criteria.json');
  assert.match(frozen.takenOn, /^[0-9a-f]{40}$/u);
  assert.equal(PHASE4_FROZEN_IDS.length, 55);
  assert.deepEqual(frozen.criteria.map((c) => c.id).sort(), [...PHASE4_FROZEN_IDS].sort());
  const live = new Set(loadRegistry(registryPath).criteria.map((c) => String(c.id)));
  for (const entry of frozen.criteria) {
    assert.ok(live.has(entry.id), `${entry.id} is not in criteria.json`);
    assert.ok(entry.checks.length > 0, `${entry.id} was frozen without its checks`);
  }
});

// A record, not a copy of itself: every entry is the base's own, read out of git. A depth-1 CI
// checkout does not hold the base, so there the test says why it skipped and the unchanged-set
// test below is the half that still runs.
test('phase4-frozen.json is the base commit’s register, entry for entry', (t) => {
  const frozen = phase4Frozen();
  try {
    execFileSync('git', ['cat-file', '-e', `${frozen.takenOn}^{commit}`], {
      cwd: repoRoot,
      stdio: 'ignore',
    });
  } catch {
    const why = `${frozen.takenOn} is not in this clone (a shallow checkout); nothing to compare`;
    console.error(`phase4-frozen: skipped — ${why}`);
    t.skip(why);
    return;
  }
  const base = JSON.parse(
    execFileSync('git', ['show', `${frozen.takenOn}:acceptance/criteria.json`], {
      cwd: repoRoot,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
  const byId = new Map(base.criteria.map((c) => [String(c.id), c]));
  for (const entry of frozen.criteria) {
    assert.deepEqual(entry, byId.get(entry.id), `${entry.id} differs from ${frozen.takenOn}`);
  }
  console.error(`phase4-frozen: ${String(frozen.criteria.length)} entries equal the base's`);
});

// The half of §49.3 that is true at every merge from here to the tag: the checks it says stand
// unchanged. The moved rows are asserted against the same file when every lane has landed, never
// here, or the first Lane-0 merge that moves its row as §49.3 requires would redden this.
const PHASE4_UNCHANGED = [
  'AC-P2-24-1',
  'AC-P2-24-1-destructive',
  'AC-P2-24-1-justified',
  'AC-P2-24-1-fixture',
  'AC-P2-24-1-reads',
  'AC-P2-24-1-remote-url',
  'AC-P2-24-3',
  'AC-P2-20-2',
  'AC-P2-20-2-denylist',
  'AC-P2-24-4',
  'AC-P2-24-4-filters',
  'AC-P2-24-4-enumeration',
  'AC-P2-24-5',
  'AC-P2-24-13',
  'AC-P2-24-15',
  'AC-P2-24-16-shallow',
  'AC-P2-25-11-producer',
  'AC-P2-25-11-unknown',
  'AC-P3-30-13-variant',
  'AC-P3-34-15',
  'AC-44-token',
  'AC-44-readonly-argv',
  'AC-44-no-destructive-git',
];
// §49.3: these pass over five sort keys "with no edit to their assertions"; the rest of the check
// may move with the lane that re-measures them.
const PHASE4_ASSERT_UNCHANGED = ['AC-P3-35-4', 'AC-P3-35-5', 'AC-P3-35-6', 'AC-P3-35-9'];

test('what §49.3 leaves unchanged stays byte-identical to the frozen copy', () => {
  const checksOf = (criteria) =>
    new Map(criteria.flatMap((c) => c.checks).map((k) => [String(k.id), k]));
  const frozen = checksOf(phase4Frozen().criteria);
  const live = checksOf(loadRegistry(registryPath).criteria);
  let compared = 0;
  for (const id of PHASE4_UNCHANGED) {
    assert.ok(frozen.has(id), `${id} is not in the frozen copy`);
    assert.deepEqual(live.get(id), frozen.get(id), `${id} has moved, and §49.3 says it stands`);
    compared += 1;
  }
  for (const id of PHASE4_ASSERT_UNCHANGED) {
    assert.ok(frozen.has(id), `${id} is not in the frozen copy`);
    assert.equal(live.get(id)?.assert, frozen.get(id).assert, `${id}'s assert has been edited`);
    compared += 1;
  }
  console.error(`phase4-frozen: ${String(compared)} unchanged checks compared`);
  assert.ok(compared > 0, 'a comparison of nothing proves nothing');
});

test('the shipped register and the shipped rule files complete without a problem', () => {
  const registry = loadRegistry(registryPath);
  const read = (name) =>
    JSON.parse(readFileSync(fileURLToPath(new URL(`../../acceptance/${name}`, import.meta.url))));
  assert.deepEqual(
    validatePhase2Complete(registry, {
      callsites: read('callsites.json'),
      forbidden: read('forbidden.json'),
    }),
    [],
  );
});

// ---------------------------------------------------------------------------------------------
// [p3-36] The phase-3 register, section by section. A registered section holds all of its
// criteria or this says which are missing; the count is `PHASE3_SECTIONS`', never restated here.
// ---------------------------------------------------------------------------------------------

/** The shipped entries of one phase-3 section: bare ids by number, lettered ids apart. */
function shippedSection(section) {
  const entries = loadRegistry(registryPath).criteria.filter((c) =>
    String(c.id).startsWith(`P3-${section}-`),
  );
  const bare = entries
    .filter((c) => !/[a-c]$/u.test(c.id))
    .map((c) => Number.parseInt(c.id.slice(`P3-${section}-`.length), 10))
    .sort((a, b) => a - b);
  const lettered = entries
    .filter((c) => /[a-c]$/u.test(c.id))
    .map((c) => c.id)
    .sort();
  return { entries, bare, lettered };
}

/**
 * Contiguous over `PHASE3_SECTIONS`' range, the ruled lettered ids and no other, every entry citing
 * its section's criteria block in its section's group, and every check a test that ran, owned by
 * the phase-3 plan that landed it. The three live observations are the only checks with no test.
 */
function assertShippedSection(section, { spec, group, lettered = [] }) {
  const { entries, bare, lettered: held } = shippedSection(section);
  const range = Array.from({ length: PHASE3_SECTIONS[section] }, (_, i) => i + 1);
  const missing = range.filter((n) => !bare.includes(n));
  assert.deepEqual(missing, [], `§${String(section)} is missing ${missing.join(', ')}`);
  assert.deepEqual(bare, range, `§${String(section)} holds an id outside its range or twice`);
  assert.deepEqual(held, lettered);
  for (const entry of entries) {
    assert.equal(entry.spec, spec, `${entry.id} cites its section's criteria block`);
    assert.equal(entry.group, group, `${entry.id} is ${group}`);
    assert.ok(entry.checks.length > 0, `${entry.id} carries no check`);
    for (const check of entry.checks) {
      assert.match(String(check.owner), /^p3-\d{2}$/u, `${check.id} names the plan that landed it`);
      if (check.deferral === 'live-observation') continue;
      assert.equal(check.status, 'automated', `${check.id} names a test that ran`);
      assert.ok(String(check.test ?? '').length > 0, `${check.id} names no test`);
    }
  }
}

// A cargo `test` is `<binary>::<function>`, and a join key one character off reads as "not run"
// only against a capture. This reads the tree instead, so a typo fails here without one.
test('every phase-3 cargo check names a function its test file declares', () => {
  const core = fileURLToPath(new URL('../../core/tests/', import.meta.url));
  const checks = loadRegistry(registryPath)
    .criteria.filter((c) => phaseOf(c.id) === 3)
    .flatMap((c) => c.checks)
    .filter((k) => k.runner === 'cargo' && String(k.test).split('::').length === 2);
  console.error(`phase-3 cargo checks read against the tree: ${String(checks.length)}`);
  assert.ok(checks.length > 0, 'a run that read no check against the tree proves nothing');
  for (const check of checks) {
    const [binary, fn] = String(check.test).split('::');
    const source = readFileSync(`${core}${binary}.rs`, 'utf8');
    assert.ok(
      new RegExp(`^\\s*(async )?fn ${fn}\\(`, 'mu').test(source),
      `${check.id}: core/tests/${binary}.rs declares no fn ${fn}`,
    );
  }
});

/** One Rust test function's text, from its `fn` line to the next top-level item. */
function rustFn(file, fn) {
  const source = readFileSync(fileURLToPath(new URL(`../../${file}`, import.meta.url)), 'utf8');
  const start = source.search(new RegExp(`^fn ${fn}\\(`, 'mu'));
  assert.ok(start >= 0, `${file} declares no fn ${fn}`);
  const rest = source.slice(start + 1);
  const end = rest.search(/^(?:#\[|\/\/\/|fn |pub |struct |impl |const |mod )/mu);
  return end === -1 ? rest : rest.slice(0, end);
}

// [Task 1] §28's eighteen and R139's P3-28-18a — the one task that moves §36.1's total to 146.
test('§28 holds its eighteen criteria and P3-28-18a', () => {
  assertShippedSection(28, { spec: '§28.12', group: 'functional', lettered: ['P3-28-18a'] });
});

test('a register holding P3-28-18a without P3-28-18 is a problem', () => {
  const lone = validatePhase3Complete({
    version: 1,
    criteria: loadRegistry(registryPath).criteria.filter((c) => c.id !== 'P3-28-18'),
  });
  assert.ok(
    lone.some((p) => p.includes('P3-28-18a is registered and P3-28-18 is not')),
    lone.join('\n'),
  );
});

// [Task 2] §29's twenty-nine, three of them in the form a ruling amended.
test('§29 holds its twenty-nine criteria, every one automated', () => {
  assertShippedSection(29, { spec: '§29.14', group: 'functional' });
});

test('§29 registers the three amended criteria in their ruled form, read off the test bodies', () => {
  // R132/F12: the fixture is read from the Rust constant and compared against it, not a literal.
  const exhaustive = rustFn(
    'core/tests/acceptance_content_scan.rs',
    'ac_p3_29_13_the_presence_predicates_are_exhaustive',
  );
  assert.match(exhaustive, /0\.\.ARCHETYPE_SAMPLE\b/u);
  assert.match(exhaustive, /> ARCHETYPE_SAMPLE\b/u);
  assert.doesNotMatch(exhaustive, /\b4_?000\b/u, 'a copied 4,000 is §36.2 rule 8 again');
  // R132/F16: the set, printed, and no numeral stating its size.
  const vocab = rustFn(
    'core/tests/acceptance_content_scan.rs',
    'ac_p3_29_21_the_job_vocabularies_stay_disjoint_and_complete',
  );
  assert.match(vocab, /JobKind::ALL/u);
  assert.match(vocab, /eprintln!/u);
  assert.doesNotMatch(vocab, /len\(\),\s*\d/u, 'the size is the type’s, never the test’s');
  // R127.3: the assertion is `present`, kept, and the unreadable-fixture justification is gone.
  const readme = rustFn(
    'core/tests/acceptance_content_scan.rs',
    'ac_p3_29_26_an_unreadable_readme_is_not_a_missing_one',
  );
  assert.match(readme, /PresenceState::Present/u);
  assert.match(readme, /PresenceState::Absent/u);
});

// [Task 3] §30's eighteen and the one lettered id §36.1 names, and the three criteria R140 split
// across waves: each is two checks with two owners, never one `deferred` check.
test('§30 holds its eighteen criteria and P3-30-11a', () => {
  assertShippedSection(30, { spec: '§30.13', group: 'functional', lettered: ['P3-30-11a'] });
});

test('P3-30-19 exists nowhere: the index minted it, withdrew it and ruled P3-28-18a instead', () => {
  const registry = loadRegistry(registryPath);
  assert.ok(!registry.criteria.some((c) => c.id === 'P3-30-19'));
  assert.ok(!registry.criteria.flatMap((c) => c.checks).some((k) => /^AC-P3-30-19\b/u.test(k.id)));
});

test('each of R140’s three split criteria carries two checks, two owners and no deferral', () => {
  const registry = loadRegistry(registryPath);
  for (const id of ['P3-30-11', 'P3-30-16', 'P3-30-18']) {
    const entry = registry.criteria.find((c) => c.id === id);
    assert.ok(entry, `${id} is registered`);
    const owners = new Set(entry.checks.map((k) => k.owner));
    const tests = new Set(entry.checks.map((k) => k.test));
    assert.ok(owners.size >= 2, `${id} is owned by one plan; R140 splits it across two`);
    assert.equal(tests.size, entry.checks.length, `${id} joins two checks to one test`);
    for (const check of entry.checks) assert.equal(check.status, 'automated', check.id);
  }
});

// [Task 5] §32's twenty-three, and the three readings §36.6 says no fixture can take. Each rides a
// criterion whose first check is automated; only the values are unobserved.
test('§32 holds its twenty-three criteria', () => {
  assertShippedSection(32, { spec: '§32.20', group: 'functional' });
});

test('exactly the three phase-3 live observations are registered, each with no test', () => {
  const checks = loadRegistry(registryPath)
    .criteria.filter((c) => phaseOf(c.id) === 3)
    .flatMap((c) => c.checks);
  const live = checks.filter((k) => k.deferral === 'live-observation');
  assert.deepEqual(
    live.map((k) => k.id).sort(),
    LIVE_OBSERVATION_CHECKS.filter((id) => id.startsWith('AC-P3-')).sort(),
  );
  for (const check of live) {
    assert.equal(check.status, 'deferred', check.id);
    assert.equal(check.test, undefined, `${check.id}: a test would be an assertion`);
    assert.deepEqual(
      Object.keys(check.verification ?? {}).sort(),
      ['evidence', 'recordedAt'],
      `${check.id} carries its verification record`,
    );
  }
});

// [Task 4] §31's eighteen. AC-P3-31-18 is registered in R127.2's corrected form or not at all.
test('§31 holds its eighteen criteria, every one automated', () => {
  assertShippedSection(31, { spec: '§31.11', group: 'functional' });
});

// [Task 6] §33's twelve, over a rendered element, so `surfaces`.
test('§33 holds its twelve criteria, every one automated', () => {
  assertShippedSection(33, { spec: '§33.11', group: 'surfaces' });
});

// [Task 7] §34's seventeen. The clamp gate is the `script` runner reading the checker's own
// capture, and AC-P3-34-12 is registered only against a test that tells all three merge classes
// apart (R129/F8).
test('§34 holds its seventeen criteria, every one automated', () => {
  assertShippedSection(34, { spec: '§34.10', group: 'surfaces' });
  const clamp = loadRegistry(registryPath)
    .criteria.find((c) => c.id === 'P3-34-14')
    ?.checks.find((k) => k.id === 'AC-P3-34-14');
  assert.equal(clamp?.runner, 'script');
  assert.equal(clamp?.test, 'check-motion-clamp:every-animated-class-is-clamped');
});

// [Task 8] §35's nine, and the two markings a phase-3 register with none of has stopped reading:
// §36.2 rule 7's cross-language mirror and a scan that prints its count.
test('§35 holds its nine criteria, and phase 3 carries a mirror and a scanning check', () => {
  assertShippedSection(35, { spec: '§35.11', group: 'surfaces' });
  const checks = loadRegistry(registryPath)
    .criteria.filter((c) => phaseOf(c.id) === 3)
    .flatMap((c) => c.checks);
  assert.ok(
    checks.some((k) => k.mirror !== undefined),
    'no phase-3 check reads both sides',
  );
  assert.ok(
    checks.some((k) => k.scanning === true),
    'no phase-3 check prints what it scanned',
  );
  assert.ok(
    loadRegistry(registryPath)
      .criteria.filter((c) => c.id.startsWith('P3-35-'))
      .flatMap((c) => c.checks)
      .some((k) => k.mirror !== undefined),
    '§35 adds the first new sort key since the cursor pair was written, and asserts it both sides',
  );
});
