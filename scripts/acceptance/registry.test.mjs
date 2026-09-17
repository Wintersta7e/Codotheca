import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  DEFERRALS,
  LIVE_OBSERVATION_CHECKS,
  PHASE2_SECTIONS,
  criterionOf,
  loadRegistry,
  phaseOf,
  rollUp,
  validatePhase2Complete,
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

// ---------------------------------------------------------------------------------------------
// The second phase. The phase-1 forms above are untouched on purpose: a second id form is added
// beside the first rather than replacing it, so every assertion in this file still reads the
// behaviour phase 1 shipped with.
// ---------------------------------------------------------------------------------------------

const p2Check = (over = {}) => ({
  id: 'AC-P2-20-1',
  status: 'deferred',
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

test('the section table is §26.1 and adds up to 102', () => {
  assert.deepEqual(PHASE2_SECTIONS, { 20: 13, 21: 15, 22: 13, 23: 12, 24: 23, 25: 26 });
  assert.equal(
    Object.values(PHASE2_SECTIONS).reduce((a, b) => a + b, 0),
    102,
  );
  assert.deepEqual(DEFERRALS, ['plan', 'live-observation']);
  assert.deepEqual(LIVE_OBSERVATION_CHECKS, ['AC-P2-20-13', 'AC-P2-21-3-floor']);
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
test('registering a phase-2 section leaves phase 1 at 70 and 171', () => {
  const registry = loadRegistry(registryPath);
  assert.deepEqual(validateRegistry(registry, repoRoot), []);
  const phase1 = registry.criteria.filter((c) => phaseOf(c.id) === 1);
  assert.equal(phase1.length, 70);
  assert.equal(
    phase1.reduce((n, c) => n + c.checks.length, 0),
    171,
  );
  // §20 owns thirteen, §23 twelve and §25 twenty-six, which is what `PHASE2_SECTIONS` declares
  // for each. Raised by this lane's own delta, read from the branch base — never to a running
  // total a later lane would have to guess at.
  assert.equal(registry.criteria.filter((c) => phaseOf(c.id) === 2).length, 51);
});
