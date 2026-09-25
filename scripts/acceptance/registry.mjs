/**
 * The acceptance registry: one entry per criterion, one or more checks per entry.
 *
 * A check's status is the whole point of the file. `automated` means a test runs today and
 * gates the build; `deferred` means the named plan lands the named test; `manual` names a
 * review gate and states why a machine cannot do it; `unmeasurable` means there is no honest
 * event pair or no budget to gate on; `external` is criterion 29 and nothing else.
 *
 * Two phases share one register. A phase-1 criterion is a §16 number (`14`, `45b`); a phase-2
 * criterion is `P2-<section>-<n>`, so **the id is the ownership map** and the phase is computed
 * from it rather than stored beside it. The phase-1 forms below are not replaced — a second id
 * form is added beside the first, so phase-1 behaviour is byte-identical.
 */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

export const STATUSES = ['automated', 'deferred', 'manual', 'unmeasurable', 'external'];
export const GROUPS = [
  'functional',
  'performance',
  'honesty',
  'distribution',
  'external',
  'surfaces',
  'subsystems',
];
export const RUNNERS = ['cargo', 'vitest', 'e2e', 'node', 'script', 'perf', 'manual', 'none'];

const CHECK_ID_P1 = /^AC-(\d{1,2}[a-c]?)(-[a-z0-9]+)*$/u;
// `AC-P2-<section>-<n>`. The slug segment must begin with a letter, or `AC-P2-25-10-ddl` is
// ambiguous with a criterion `P2-25-1` carrying a slug `0-ddl`.
const CHECK_ID_P2 = /^AC-(P2-2[0-5]-\d{1,2})(-[a-z][a-z0-9]*)*$/u;
const P2_ID = /^P2-(2[0-5])-(\d{1,2})$/u;
// §36.1's `AC-P3-<section>-<n>`. `2[89]|3[0-5]` is an assertion rather than a convenience: §36
// owns no criterion of its own, so a `P3-36-*` id is refused by the id form itself instead of by
// a reviewer — the same reason §26 is excluded from `2[0-5]`. The numeric segment carries the
// optional letter, because §30 holds `P3-30-11` **and** `P3-30-11a` and both are criteria; the
// slug segment keeps phase 2's begins-with-a-letter rule, or `AC-P3-32-16-caps` is ambiguous with
// a criterion `P3-32-1` carrying a slug `6-caps`.
const CHECK_ID_P3 = /^AC-(P3-(?:2[89]|3[0-5])-\d{1,2}[a-c]?)(-[a-z][a-z0-9]*)*$/u;
const P3_ID = /^P3-(2[89]|3[0-5])-(\d{1,2}[a-c]?)$/u;
// §49.1's `AC-P4-<section>-<n>`. `3[89]|4[0-8]` refuses §37 and §49, which own no criterion, by
// the id form rather than a reviewer. No letter: phase 4 has none, and one needs a ruling (R139).
const CHECK_ID_P4 = /^AC-(P4-(?:3[89]|4[0-8])-\d{1,2})(-[a-z][a-z0-9]*)*$/u;
const P4_ID = /^P4-(3[89]|4[0-8])-(\d{1,2})$/u;
// R44: a plan number, optionally with the letter suffix of a second half — `13`, `13b`, `13c`
// — and a phase-2, phase-3 or phase-4 plan id, `p2-20`, `p3-36a`, `p4-46b`. Phase 4's three
// Lane-0 plans carry a capital `L` (`p4-L0a`), so widening the prefix alone refused all three.
//
// §36.1 says the owning plan is *"deliberately absent"* and this requires one on every `deferred`
// check. Both are right: an `owner` field is absent from §36.1's **register table**, and a
// `deferred` check still names the plan that will discharge it. The table is the ownership map
// for criteria; the owner field is the discharge record for a check that has not landed.
const OWNER = /^(?:(?:p[234]-)?\d{2}[a-c]?|p4-L0[a-c])$/u;
const GATE = /^[A-Z][A-Z0-9-]+$/u;
const WEAKEST = ['automated', 'deferred', 'manual', 'unmeasurable', 'external'];
const LETTERED = { 45: ['45a', '45b', '45c'], 48: ['48a', '48b'] };

/**
 * §26.1's table. `2[0-5]` is an assertion rather than a convenience: §26 owns no criterion of
 * its own, so a `P2-26-*` id is refused by the id form itself instead of by a reviewer.
 */
export const PHASE2_SECTIONS = { 20: 13, 21: 17, 22: 13, 23: 12, 24: 23, 25: 26 };

/**
 * §36.1's table, and it is the **contiguous range** a section holds, never the section's total.
 * The completeness loop walks `1..PHASE3_SECTIONS[section]` and reports every `n` with no hit, so
 * `28: 19` would demand a `P3-28-19` that R139 says does not exist and the register could never
 * validate. §36.1's per-section totals are 19 · 29 · 19 · 18 · 23 · 12 · 17 · 9 = 146; this is
 * 18 · 29 · 18 · 18 · 23 · 12 · 17 · 9 = 144, and `LETTERED_P3`'s two ids are the difference.
 *
 * Duplicated from §36.1 rather than derived from it: `.dev/spec/` is gitignored and does not
 * exist in a fresh clone or on CI, so a validator that read the section would scan nothing and
 * pass on nothing. Deriving the counts from the register instead is worse — the table would then
 * agree with the register by construction. The copy is made safe by the two rules around it: a
 * registered section holds all of its criteria contiguous, and the sum is asserted against 146.
 */
export const PHASE3_SECTIONS = { 28: 18, 29: 29, 30: 18, 31: 18, 32: 23, 33: 12, 34: 17, 35: 9 };

/**
 * Every phase-3 id carrying a letter. Phase 1's lettered rule does not transfer: criterion 45
 * appears *only* in lettered form and has no bare id, while §28 holds `P3-28-18` **and**
 * `P3-28-18a` and §30 holds `P3-30-11` **and** `P3-30-11a`. The letter is an additional id beside
 * the bare one and neither stands in for the other, so the rule is written over this list rather
 * than over one id — it has two subjects today and the next ruling adds a third without touching
 * the validator.
 */
export const LETTERED_P3 = ['P3-28-18a', 'P3-30-11a'];

/**
 * §49.1's table, copied for §36.5's reason (`.dev/spec/` is absent on CI); summed once, at 253, in
 * `registry.test.mjs`. Phase 4 has no letter, so the range and the total are the same number.
 */
export const PHASE4_SECTIONS = {
  38: 24,
  39: 13,
  40: 22,
  41: 16,
  42: 25,
  43: 22,
  44: 30,
  45: 22,
  46: 30,
  47: 22,
  48: 27,
};

/**
 * R207: no capture reads a Windows-native result — CI grades acceptance on Ubuntu and the gate
 * copies the WSL cargo run — so a check naming Windows is a recorded manual gate, never an
 * automated claim a Linux run would grade without executing it.
 */
const PLATFORMS = ['linux', 'windows'];
const WINDOWS_GATE = 'WINDOWS-NATIVE';

/**
 * A deferral says *what kind of thing has not happened yet*. `plan` is phase 1's meaning and
 * the default, so all 121 phase-1 deferrals are unchanged. `live-observation` is documentation
 * knowledge until it is checked against a live response, and it **refuses a test id** — a test
 * standing in for an observation is exactly the assertion this status exists to refuse.
 */
export const DEFERRALS = ['plan', 'live-observation'];

/**
 * The ids ruled live-observation, and nothing else. Another cannot be added without a ruling,
 * which is the point: the status exists to be honest about known gaps, not to become a place to
 * put anything inconvenient. R56 ruled the first two; §36.6 rules the three phase-3 ones, one per
 * subject it names — whether the advisories endpoint returns a rate-limit header at all, what
 * that header says the resource is, and §32's 16 MB / 32-lockfile caps. Each is a **second**
 * check on a criterion whose first check is automated, so nothing is registered as observed and
 * nothing loses its fixture test; only the values are unobserved.
 */
export const LIVE_OBSERVATION_CHECKS = [
  'AC-P2-20-13',
  'AC-P2-21-3-floor',
  'AC-P3-32-3-header',
  'AC-P3-32-3-resource',
  'AC-P3-32-16-caps',
];

// A number's source is one of three shapes, so "is this a source or a restatement of the
// figure?" is decidable. A heuristic over the assert text would fire on every id and every
// year and would be tuned until it stopped firing, which is the defect the rule exists for.
const SOURCE_SPEC = /^§\d{1,2}\.\d/u;
const SOURCE_PROBE = /^probe:[a-z0-9][a-z0-9-]*$/u;

export function criterionOf(checkId) {
  const id = String(checkId);
  const p1 = CHECK_ID_P1.exec(id);
  if (p1 !== null) return p1[1];
  const p2 = CHECK_ID_P2.exec(id);
  if (p2 !== null) return p2[1];
  const p3 = CHECK_ID_P3.exec(id);
  if (p3 !== null) return p3[1];
  const p4 = CHECK_ID_P4.exec(id);
  return p4 === null ? null : p4[1];
}

/**
 * Accepts a criterion id or a check id, because the phase is in the id either way. §36.5 asks for
 * *"a phase field"*; there is none and there must not be one — the id is the ownership map and
 * the phase is read back out of it rather than stored beside it.
 */
const PHASE_PREFIX = /^(?:AC-)?P(\d+)-/u;
export function phaseOf(id) {
  const m = PHASE_PREFIX.exec(String(id));
  return m === null ? 1 : Number.parseInt(m[1], 10);
}

function sectionOf(criterionId) {
  const id = String(criterionId);
  const p2 = P2_ID.exec(id);
  if (p2 !== null) return p2[1];
  const p3 = P3_ID.exec(id);
  if (p3 !== null) return p3[1];
  const p4 = P4_ID.exec(id);
  return p4 === null ? null : p4[1];
}

export function rollUp(entry) {
  let worst = 0;
  for (const check of entry.checks) worst = Math.max(worst, WEAKEST.indexOf(check.status));
  return WEAKEST[worst];
}

export function loadRegistry(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function deferredProblems(where, check, problems) {
  const deferral = check.deferral ?? 'plan';
  if (!DEFERRALS.includes(deferral)) {
    problems.push(`${where}: deferral is not one of ${DEFERRALS.join(', ')}`);
    return;
  }
  if (!OWNER.test(String(check.owner))) {
    problems.push(
      `${where}: deferred needs an owner plan number, e.g. "13", "13c", "p2-20" or "p4-L0a"`,
    );
  }
  if (deferral === 'plan') {
    if (typeof check.test !== 'string' || check.test.length === 0) {
      problems.push(`${where}: deferred needs the test id it will carry`);
    }
    return;
  }

  if (!LIVE_OBSERVATION_CHECKS.includes(String(check.id))) {
    problems.push(
      `${where}: live-observation is ${LIVE_OBSERVATION_CHECKS.join(' and ')} and nothing` +
        ' else — a third one needs a ruling, not a field',
    );
  }
  if (check.test !== undefined) {
    problems.push(`${where}: a live-observation carries no test id — a test would be an assertion`);
  }
  if (typeof check.reason !== 'string' || check.reason.length < 20) {
    problems.push(`${where}: live-observation needs a reason`);
  }
  const v = check.verification;
  if (typeof v !== 'object' || v === null) {
    problems.push(`${where}: live-observation needs verification { recordedAt, evidence }`);
    return;
  }
  if (!('recordedAt' in v) || !('evidence' in v)) {
    problems.push(
      `${where}: verification carries both recordedAt and evidence, null until observed`,
    );
    return;
  }
  if (v.recordedAt === null && v.evidence !== null) {
    problems.push(`${where}: verification.evidence with no recordedAt to date it`);
  }
  if (v.recordedAt !== null && (typeof v.evidence !== 'string' || v.evidence.length < 20)) {
    problems.push(`${where}: verification.recordedAt with no evidence — a date is not a record`);
  }
  // §49.5 item 11: an observation is of a build, and the build is a commit.
  if (v.recordedAt !== null && !FULL_COMMIT.test(String(v.commit))) {
    problems.push(`${where}: verification.recordedAt needs the commit it was observed against`);
  }
}

const FULL_COMMIT = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/u;
const RECORD_DATE = /^\d{4}-\d{2}-\d{2}$/u;

/**
 * §49.6's manual record: `{ recordedAt, evidence, commit }` with all three set, or `null` until the
 * gate runs. Only a manual check carries it, and every phase-4 manual check carries the key, so an
 * unrecorded gate reads as unrecorded rather than as a field nobody wrote. An earlier phase's
 * manual gate gains the key when it is recorded, which keeps its entry untouched until then.
 */
function recordProblems(entry, where, check, problems) {
  if (!('record' in check)) {
    if (check.status === 'manual' && phaseOf(entry.id) === 4) {
      problems.push(`${where}: a phase-4 manual check carries the record key, null until recorded`);
    }
    return;
  }
  if (check.status !== 'manual') {
    problems.push(`${where}: only a manual check carries a record`);
    return;
  }
  const r = check.record;
  if (r === null) return;
  if (typeof r !== 'object' || !('recordedAt' in r) || !('evidence' in r) || !('commit' in r)) {
    problems.push(`${where}: a record is { recordedAt, evidence, commit }, all three, or null`);
    return;
  }
  if (!RECORD_DATE.test(String(r.recordedAt))) {
    problems.push(`${where}: record.recordedAt is a YYYY-MM-DD date`);
  }
  if (typeof r.evidence !== 'string' || r.evidence.length < 20) {
    problems.push(`${where}: record.evidence is at least a sentence — a date is not a record`);
  }
  if (!FULL_COMMIT.test(String(r.commit))) {
    problems.push(`${where}: record.commit is a full 40- or 64-hex commit id`);
  }
}

/**
 * The three markings an author declares, one per acceptance rule that had no mechanism. They
 * are declared and never inferred: the validator checks the marking is well-formed, and the
 * author is the one who knows whether a check scans, mirrors or states a figure.
 */
function markingProblems(where, check, problems, root) {
  if (check.scanning !== undefined) {
    if (check.scanning !== true) {
      problems.push(`${where}: scanning is true or absent`);
    } else if (
      !/\bcount\b/iu.test(String(check.assert)) ||
      !/\bzero\b/iu.test(String(check.assert))
    ) {
      problems.push(`${where}: a scanning check's assert says it prints a count and fails at zero`);
    }
  }
  if (check.mirror !== undefined) {
    const other = check.mirror === null ? undefined : check.mirror.other;
    if (typeof other !== 'string' || other.length === 0) {
      problems.push(`${where}: mirror names the other language's file the test reads`);
    } else if (root !== null && !existsSync(join(root, other))) {
      problems.push(`${where}: mirror names ${other}, which is not in the tree`);
    }
  }
  if (check.source !== undefined) {
    const source = String(check.source);
    if (source !== 'schema' && !SOURCE_SPEC.test(source) && !SOURCE_PROBE.test(source)) {
      problems.push(
        `${where}: source is a §N.N citation, "probe:<name>" or "schema" — a figure` +
          ' restated is not its source',
      );
    }
  }
}

function phase2CheckProblems(where, check, problems) {
  if (check.runner === 'perf' || check.measurement !== undefined || check.budget !== undefined) {
    problems.push(
      `${where}: no performance sample exists and phase 2 adds none — a phase-2 check` +
        ' carries no perf runner, no measurement and no budget',
    );
  }
  // Restated rather than left to run through an id comparison a reader would have to find.
  if (check.status === 'external') {
    problems.push(`${where}: external is criterion 29 and is not reachable from phase 2`);
  }
}

function checkProblems(entry, check, seenCheckIds, problems, root) {
  const where = `${entry.id}/${String(check.id)}`;
  if (criterionOf(check.id) === null) {
    problems.push(`${where}: check id is not AC-<criterion>[-slug]`);
  } else if (criterionOf(check.id) !== entry.id) {
    problems.push(`${where}: check id names a different criterion`);
  }
  if (seenCheckIds.has(check.id)) problems.push(`${where}: duplicate check id`);
  seenCheckIds.add(check.id);
  if (!STATUSES.includes(check.status)) {
    problems.push(`${where}: status is not one of ${STATUSES.join(', ')}`);
  }
  if (!RUNNERS.includes(check.runner)) {
    problems.push(`${where}: runner is not one of ${RUNNERS.join(', ')}`);
  }
  if (typeof check.assert !== 'string' || check.assert.length < 10) {
    problems.push(`${where}: assert must say what it asserts`);
  }

  if (check.status === 'automated') {
    if (!['cargo', 'vitest', 'e2e', 'node', 'script'].includes(check.runner)) {
      problems.push(
        `${where}: an automated check runs on cargo, vitest, e2e, node or script — never perf`,
      );
    }
    if (typeof check.test !== 'string' || check.test.length === 0) {
      problems.push(`${where}: automated needs a test id`);
    }
  }
  if (check.status === 'deferred') deferredProblems(where, check, problems);
  else if (check.deferral !== undefined) {
    problems.push(`${where}: only a deferred check carries a deferral`);
  }
  if (check.status === 'manual') {
    if (!GATE.test(String(check.gate))) {
      problems.push(`${where}: manual needs a gate name in SCREAMING-KEBAB`);
    }
    if (typeof check.reason !== 'string' || check.reason.length < 20) {
      problems.push(`${where}: manual needs a reason`);
    }
    if (check.runner !== 'manual') problems.push(`${where}: a manual check has runner "manual"`);
  }
  if (check.status === 'unmeasurable') {
    if (typeof check.reason !== 'string' || check.reason.length < 20) {
      problems.push(`${where}: unmeasurable needs a reason`);
    }
    if (check.runner !== 'none') problems.push(`${where}: an unmeasurable check has runner "none"`);
    if (check.budget !== undefined) problems.push(`${where}: unmeasurable may not carry a budget`);
  }
  if (check.status === 'external') {
    if (entry.id !== '29') problems.push(`${where}: external is criterion 29 and nothing else`);
    if (typeof check.reason !== 'string' || check.reason.length < 20) {
      problems.push(`${where}: external needs a reason`);
    }
    if (check.runner !== 'none') problems.push(`${where}: an external check has runner "none"`);
  }

  markingProblems(where, check, problems, root);
  if (phaseOf(entry.id) === 2) phase2CheckProblems(where, check, problems);
  phase4FieldProblems(entry, where, check, problems);
  recordProblems(entry, where, check, problems);
}

/**
 * The two fields phase 4 adds to a check, declared by the author and never inferred. `firstTag`
 * is a copy of §49.1a's Bar column, which the release mode's `ft` run reads; `platforms` says which
 * platform's results grade the check, absent meaning Linux — the only capture there is.
 */
function phase4FieldProblems(entry, where, check, problems) {
  if (check.firstTag !== undefined) {
    if (check.firstTag !== true) problems.push(`${where}: firstTag is true or absent`);
    else if (phaseOf(entry.id) !== 4) {
      problems.push(`${where}: firstTag marks a phase-4 check that gates the first tag`);
    }
  }
  const p = check.platforms;
  if (p !== undefined) {
    if (
      !Array.isArray(p) ||
      p.length === 0 ||
      p.some((x) => !PLATFORMS.includes(x)) ||
      new Set(p).size !== p.length
    ) {
      problems.push(`${where}: platforms is a non-empty, duplicate-free subset of linux, windows`);
    } else if (
      p.includes('windows') &&
      (check.status !== 'manual' || check.gate !== WINDOWS_GATE)
    ) {
      problems.push(
        `${where}: a check naming windows is manual with gate ${WINDOWS_GATE} — no capture` +
          ' reads a Windows result, so a Linux run would grade it without running it',
      );
    }
  }
  if (check.gate === WINDOWS_GATE && (p?.length !== 1 || p[0] !== 'windows')) {
    problems.push(`${where}: a ${WINDOWS_GATE} check declares platforms ["windows"] exactly`);
  }
}

/**
 * The measurement rule binds a check that states a **number** — one on the `perf` runner, or any
 * check carrying a budget. A performance criterion also has clauses that assert behaviour and no
 * figure (J4 resumes from its cursor; the flicker obeys its eligibility), and requiring an event
 * pair from those would be satisfied by inventing one, which is the failure this rule exists to
 * prevent rather than a use of it.
 */
function statesANumber(check) {
  return check.runner === 'perf' || (check.budget ?? []).length > 0;
}

function performanceProblems(entry, check, problems) {
  const where = `${entry.id}/${String(check.id)}`;
  if (check.status === 'unmeasurable') return;
  if (!statesANumber(check)) return;
  const m = check.measurement;
  if (m === undefined || m === null) {
    problems.push(`${where}: a performance check states a measurement or is marked unmeasurable`);
    return;
  }
  if (!['latency', 'duration', 'rate', 'size'].includes(m.kind)) {
    problems.push(`${where}: measurement.kind`);
  }
  if (
    !Array.isArray(m.machines) ||
    m.machines.length === 0 ||
    m.machines.some((x) => !['A', 'B'].includes(x))
  ) {
    problems.push(`${where}: measurement.machines is a non-empty subset of ["A","B"]`);
  }
  if (!['warm', 'cold', 'n/a'].includes(m.thermal)) {
    problems.push(`${where}: measurement.thermal is warm, cold or n/a`);
  }
  if (['latency', 'duration'].includes(m.kind)) {
    if (typeof m.from !== 'string' || m.from.length === 0) {
      problems.push(`${where}: measurement.from — from which event`);
    }
    if (typeof m.to !== 'string' || m.to.length === 0) {
      problems.push(`${where}: measurement.to — to which event`);
    }
  }
  if (m.kind === 'rate' && (typeof m.run !== 'string' || m.run.length === 0)) {
    problems.push(`${where}: a rate states its run definition instead of an event pair`);
  }
  for (const b of check.budget ?? []) {
    if (!['p50', 'p95', 'max', 'missedFraction', 'bytes', 'files'].includes(b.metric)) {
      problems.push(`${where}: budget.metric`);
    }
    if (!['<', '<='].includes(b.op)) problems.push(`${where}: budget.op`);
    if (typeof b.value !== 'number') problems.push(`${where}: budget.value`);
  }
}

/**
 * Phase 1's `1..67` rule, applied per section. A section holding **at least one** criterion must
 * hold all of them, contiguous, with no gaps and no duplicates. A section holding **none** is
 * not yet registered and is silent — which is what lets the harness widening land before the
 * first phase-2 criterion exists, and lets it bite from the first one onwards.
 */
/**
 * R46, mechanically, and the reason this function takes the rule files.
 *
 * R46 found *41 checks with an owner and no implementing task* — "the structure that makes
 * something checkable gets built, and the thing itself is assumed to be somebody's next step".
 * Every phase-2 plan is merged by the time this runs, so each rule below is a way a criterion
 * could still ship as an intention.
 *
 * `rules` is `{ forbidden, callsites }`, each the parsed rule file. It is optional so a unit
 * test can validate a registry object alone; a real run passes both, and without them the
 * surviving-escape rule is not checked rather than being checked against nothing.
 */
function phase2CompletenessProblems(registry, rules) {
  const problems = [];
  const phase2 = (registry.criteria ?? []).filter((c) => phaseOf(c.id) === 2);
  let scanning = 0;
  let mirrors = 0;

  // A `test` is a join key, so two checks naming one test are two criteria reading as covered by
  // one run. Some of those are correct — a static rule narrowed in place is claimed by both
  // phases, and a criterion split across two owners takes two checks — so the share is
  // **declared**: exactly one check per test id may omit `shares`, and every other names a check
  // in the same group. A copy-pasted key declares nothing and fails here.
  const byTest = new Map();
  for (const entry of registry.criteria ?? []) {
    for (const check of entry.checks ?? []) {
      if (typeof check.test !== 'string') continue;
      byTest.set(check.test, [...(byTest.get(check.test) ?? []), check]);
    }
  }
  for (const [test, group] of byTest) {
    if (group.length === 1) continue;
    const ids = group.map((c) => String(c.id));
    const declarers = group.filter((c) => c.shares === undefined);
    if (declarers.length !== 1) {
      problems.push(
        `${ids.join(' and ')} share the test ${test} with ${String(declarers.length)} declarers` +
          ' — exactly one check per test id is the one that owns it',
      );
    }
    for (const check of group) {
      if (check.shares !== undefined && !ids.includes(String(check.shares))) {
        problems.push(
          `${String(check.id)}: shares names ${String(check.shares)}, not in its group`,
        );
      }
    }
  }

  for (const entry of phase2) {
    if ((entry.checks ?? []).length === 0) problems.push(`${String(entry.id)}: carries no check`);
    for (const check of entry.checks ?? []) {
      const where = `${String(entry.id)}/${String(check.id)}`;
      if (check.scanning === true) scanning += 1;
      if (check.mirror !== undefined) mirrors += 1;
      if (check.status === 'deferred' && (check.deferral ?? 'plan') === 'plan') {
        problems.push(
          `${where}: every phase-2 plan has merged, so a deferral to one is a deferral to nobody`,
        );
      }
    }
  }

  // Zero of either is the field never having been written, not a phase with no gate that scans
  // and no value stated on both sides. Same scans-nothing rule, turned on the register itself.
  if (phase2.length > 0 && scanning === 0) {
    problems.push('the phase-2 register holds no scanning check at all');
  }
  if (phase2.length > 0 && mirrors === 0) {
    problems.push('the phase-2 register holds no mirror at all');
  }

  // The escape is discharged by the criterion it names being **in the register**, not by the rule
  // carrying the field — which is what the message below already says. An unconditional refusal
  // is correct at the end of a phase and wrong from the start of the next: p3-36 owns
  // `criteria.json` and runs last, so every phase-3 lane that lands a static rule before its
  // criterion exists must carry the escape for five waves, and each of them would otherwise
  // redden the gate over a register it is not allowed to edit.
  const registered = new Set();
  for (const entry of registry.criteria ?? []) {
    registered.add(String(entry.id));
    for (const check of entry.checks ?? []) registered.add(String(check.id));
  }
  for (const [name, file] of Object.entries(rules ?? {})) {
    const list = file?.rules ?? [];
    if (list.length === 0) {
      problems.push(`${name}: lists no rule — the escape scan read nothing`);
      continue;
    }
    for (const rule of list) {
      if (rule.pendingRegistryEntry === undefined) continue;
      // A rule names either a criterion id (`63`, `P2-24-5`) or a check id (`AC-P2-24-3`); both
      // forms are live in the tree, so both resolve.
      const named = String(rule.pendingRegistryEntry?.criterion);
      if (registered.has(named) || registered.has(criterionOf(named) ?? named)) {
        problems.push(
          `${name}:${String(rule.id)}: still carries the escape a registered check discharges`,
        );
      }
    }
  }
  return problems;
}

export function validatePhase2Complete(registry, rules = null) {
  const problems = phase2CompletenessProblems(registry, rules);
  const bySection = new Map();
  for (const entry of registry.criteria ?? []) {
    if (phaseOf(entry.id) !== 2) continue;
    const m = P2_ID.exec(String(entry.id));
    if (m === null) continue; // the id form already reported it
    bySection.set(m[1], [...(bySection.get(m[1]) ?? []), Number.parseInt(m[2], 10)]);
  }
  for (const section of [...bySection.keys()].sort()) {
    const ns = bySection.get(section);
    const expected = PHASE2_SECTIONS[section];
    // The table is the owner of how many criteria a section holds, and a section it does not
    // name has no count to be complete against. Without this the loop below runs `n <= undefined`
    // — never once — and `n > undefined` is false, so a section missing from the table would
    // validate as **complete and silent**. Today `P2_ID` keeps that unreachable; `P2_ID` and this
    // table are two statements of the same `2[0-5]`, so it is one regex edit away.
    if (expected === undefined) {
      problems.push(`P2-${section}: §${section} is not in PHASE2_SECTIONS and owns no criterion`);
      continue;
    }
    const held = `§${section} holds ${String(ns.length)} of its ${String(expected)} criteria`;
    for (let n = 1; n <= expected; n += 1) {
      const hits = ns.filter((x) => x === n).length;
      if (hits === 0) problems.push(`P2-${section}-${String(n)} is missing: ${held}`);
      else if (hits > 1) problems.push(`P2-${section}-${String(n)} appears ${String(hits)} times`);
    }
    for (const n of ns) {
      if (n < 1 || n > expected) {
        problems.push(`P2-${section}-${String(n)} is outside §${section}'s 1..${String(expected)}`);
      }
    }
  }
  return problems;
}

/**
 * Phase 3's own completeness rules. A third function rather than a parameterised one, for the
 * reason the second exists: each phase's rules are its own ruling set, and folding them makes a
 * phase-2 rule silently govern phase 3.
 *
 * It takes no rule files. The surviving-escape scan is phase-agnostic and belongs to
 * `phase2CompletenessProblems`, which already runs on every real gate; reading the rule files
 * from here too would report every survivor twice.
 */
export function validatePhase3Complete(registry) {
  const problems = [];
  const phase3 = (registry.criteria ?? []).filter((c) => phaseOf(c.id) === 3);
  const ids = new Set(phase3.map((c) => String(c.id)));
  let scanning = 0;
  let mirrors = 0;

  for (const entry of phase3) {
    const id = String(entry.id);
    if ((entry.checks ?? []).length === 0) problems.push(`${id}: carries no check`);
    for (const check of entry.checks ?? []) {
      const where = `${id}/${String(check.id)}`;
      if (check.scanning === true) scanning += 1;
      if (check.mirror !== undefined) mirrors += 1;
      // p3-36 registers the criteria and runs last, so every phase-3 plan has merged by the time
      // an entry exists at all.
      if (check.status === 'deferred' && (check.deferral ?? 'plan') === 'plan') {
        problems.push(
          `${where}: every phase-3 plan has merged, so a deferral to one is a deferral to nobody`,
        );
      }
      // The one performance question phase 3 raises is D9's gate, and it is asked of criteria 22
      // and 30 — phase-1 ids that already carry the instrument's definition. A phase-3 criterion
      // that needs a budget has been written in the wrong place.
      if (
        check.runner === 'perf' ||
        check.measurement !== undefined ||
        check.budget !== undefined
      ) {
        problems.push(
          `${where}: no performance sample exists and phase 3 adds none — a phase-3 check` +
            ' carries no perf runner, no measurement and no budget',
        );
      }
      // Restated rather than left to run through an id comparison a reader would have to find.
      if (check.status === 'external') {
        problems.push(`${where}: external is criterion 29 and is not reachable from phase 3`);
      }
    }
  }

  // Zero of either is the field never having been written, not a phase with no gate that scans
  // and no value stated on both sides. Same scans-nothing rule, turned on the register itself.
  if (phase3.length > 0 && scanning === 0) {
    problems.push('the phase-3 register holds no scanning check at all');
  }
  if (phase3.length > 0 && mirrors === 0) {
    problems.push('the phase-3 register holds no mirror at all');
  }

  const bySection = new Map();
  for (const entry of phase3) {
    const m = P3_ID.exec(String(entry.id));
    if (m === null) continue; // the id form already reported it
    // A lettered id must not reach `ns`: `Number.parseInt('18a', 10)` is **18**, so folding one
    // in makes its bare twin report a duplicate that exists in no register. The suffixed ids are
    // validated against `LETTERED_P3` below instead.
    if (/[a-c]$/u.test(m[2])) continue;
    bySection.set(m[1], [...(bySection.get(m[1]) ?? []), Number.parseInt(m[2], 10)]);
  }
  for (const section of [...bySection.keys()].sort()) {
    const ns = bySection.get(section);
    const expected = PHASE3_SECTIONS[section];
    // Without this the loop below runs `n <= undefined` — never once — and a section missing from
    // the table would validate as **complete and silent**.
    if (expected === undefined) {
      problems.push(`P3-${section}: §${section} is not in PHASE3_SECTIONS and owns no criterion`);
      continue;
    }
    const held = `§${section} holds ${String(ns.length)} of its ${String(expected)} criteria`;
    for (let n = 1; n <= expected; n += 1) {
      const hits = ns.filter((x) => x === n).length;
      if (hits === 0) problems.push(`P3-${section}-${String(n)} is missing: ${held}`);
      else if (hits > 1) problems.push(`P3-${section}-${String(n)} appears ${String(hits)} times`);
    }
    for (const n of ns) {
      if (n < 1 || n > expected) {
        problems.push(`P3-${section}-${String(n)} is outside §${section}'s 1..${String(expected)}`);
      }
    }
  }

  // Both directions, and written over the list rather than over one id. §28 holds `-18` **and**
  // `-18a`, §30 holds `-11` **and** `-11a`, and neither stands in for the other — unlike phase
  // 1's 45, which has no bare form at all.
  for (const lettered of LETTERED_P3) {
    const bare = lettered.slice(0, -1);
    if (ids.has(lettered) && !ids.has(bare)) {
      problems.push(`${lettered} is registered and ${bare} is not — the letter is an extra id`);
    }
    if (ids.has(bare) && !ids.has(lettered)) {
      problems.push(`${bare} is registered and ${lettered} is not — §36.1 holds both`);
    }
  }
  return problems;
}

/**
 * Phase 4's own completeness rules, a fourth function for the reason the third exists: each
 * phase's rules are its own ruling set.
 *
 * **Unlike phases 2 and 3 it permits a `deferred` check owned by a plan.** Phase 4 registers all of
 * its criteria before its lanes land (so each lane's first test finds its check), and the release
 * mode refuses every such deferral at the tag instead.
 */
export function validatePhase4Complete(registry) {
  const problems = [];
  const phase4 = (registry.criteria ?? []).filter((c) => phaseOf(c.id) === 4);
  let scanning = 0;
  let mirrors = 0;

  for (const entry of phase4) {
    const id = String(entry.id);
    if ((entry.checks ?? []).length === 0) problems.push(`${id}: carries no check`);
    for (const check of entry.checks ?? []) {
      const where = `${id}/${String(check.id)}`;
      if (check.scanning === true) scanning += 1;
      if (check.mirror !== undefined) mirrors += 1;
      if (
        check.runner === 'perf' ||
        check.measurement !== undefined ||
        check.budget !== undefined
      ) {
        problems.push(
          `${where}: no performance sample exists and phase 4 adds none — a phase-4 check` +
            ' carries no perf runner, no measurement and no budget',
        );
      }
      if (check.status === 'external') {
        problems.push(`${where}: external is criterion 29 and is not reachable from phase 4`);
      }
    }
  }

  // Zero of either is the field never having been written, not a phase with no gate that scans
  // and no value stated on both sides.
  if (phase4.length > 0 && scanning === 0) {
    problems.push('the phase-4 register holds no scanning check at all');
  }
  if (phase4.length > 0 && mirrors === 0) {
    problems.push('the phase-4 register holds no mirror at all');
  }

  const bySection = new Map();
  for (const entry of phase4) {
    const m = P4_ID.exec(String(entry.id));
    if (m === null) continue; // the id form already reported it
    bySection.set(m[1], [...(bySection.get(m[1]) ?? []), Number.parseInt(m[2], 10)]);
  }
  for (const section of [...bySection.keys()].sort()) {
    const ns = bySection.get(section);
    const expected = PHASE4_SECTIONS[section];
    // Without this the loop below runs `n <= undefined` — never once — and a section missing from
    // the table would validate as **complete and silent**.
    if (expected === undefined) {
      problems.push(`P4-${section}: §${section} is not in PHASE4_SECTIONS and owns no criterion`);
      continue;
    }
    const held = `§${section} holds ${String(ns.length)} of its ${String(expected)} criteria`;
    for (let n = 1; n <= expected; n += 1) {
      const hits = ns.filter((x) => x === n).length;
      if (hits === 0) problems.push(`P4-${section}-${String(n)} is missing: ${held}`);
      else if (hits > 1) problems.push(`P4-${section}-${String(n)} appears ${String(hits)} times`);
    }
    for (const n of ns) {
      if (n < 1 || n > expected) {
        problems.push(`P4-${section}-${String(n)} is outside §${section}'s 1..${String(expected)}`);
      }
    }
  }
  return problems;
}

/**
 * `root` is optional so a unit test can validate a registry object in isolation. Pass it from
 * any real run: without it a `mirror` is checked for shape and not for the file it names.
 */
export function validateRegistry(registry, root = null) {
  const problems = [];
  if (registry.version !== 1) problems.push('registry.version must be 1');
  const criteria = Array.isArray(registry.criteria) ? registry.criteria : [];
  if (criteria.length === 0) problems.push('registry.criteria is empty');

  const seenIds = new Set();
  const seenCheckIds = new Set();
  const integers = new Set();

  for (const entry of criteria) {
    const id = String(entry.id);
    if (seenIds.has(id)) problems.push(`${id}: duplicate criterion id`);
    seenIds.add(id);
    // The phase-1 arm is a branch of its own and no longer the `else`. A `P3-` id fell through to
    // it, was read by `Number.parseInt` as NaN, and came back as `criterion id outside 1..67`
    // **and** `spec must cite §16.` — two problems naming the wrong defect, which is how the last
    // two widenings were mistaken for malformed data.
    const phase = phaseOf(id);
    const section = phase === 1 ? '16' : sectionOf(id);
    if (phase === 1) {
      const n = Number.parseInt(id, 10);
      if (!Number.isInteger(n) || n < 1 || n > 67) {
        problems.push(`${id}: criterion id outside 1..67`);
      } else integers.add(n);
      const letter = id.slice(String(n).length);
      if (letter !== '' && !(LETTERED[n] ?? []).includes(id)) {
        problems.push(`${id}: only 45a/b/c and 48a/b carry a letter`);
      }
      if (letter === '' && LETTERED[n] !== undefined) {
        problems.push(`${id}: criterion ${String(n)} appears only in lettered form`);
      }
    } else if (phase === 2) {
      if (section === null) problems.push(`${id}: a phase-2 criterion id is P2-<20..25>-<n>`);
    } else if (phase === 3) {
      if (section === null) problems.push(`${id}: a phase-3 criterion id is P3-<28..35>-<n>`);
      else if (/[a-c]$/u.test(id) && !LETTERED_P3.includes(id)) {
        problems.push(`${id}: only ${LETTERED_P3.join(' and ')} carry a letter`);
      }
    } else if (phase === 4) {
      if (section === null) problems.push(`${id}: a phase-4 criterion id is P4-<38..48>-<n>`);
    } else {
      problems.push(`${id}: names phase ${String(phase)}, and the register holds 1, 2, 3 and 4`);
    }
    if (!GROUPS.includes(entry.group)) {
      problems.push(`${id}: group is not one of ${GROUPS.join(', ')}`);
    }
    if (typeof entry.title !== 'string' || entry.title.length === 0) problems.push(`${id}: title`);
    // An id whose phase the register does not hold has no section to cite, and `§16.` would be
    // the same wrong defect twice.
    const cites = section === null ? null : `§${section}.`;
    if (cites !== null && (typeof entry.spec !== 'string' || !entry.spec.startsWith(cites))) {
      problems.push(`${id}: spec must cite ${cites}<n>`);
    }
    if (!Array.isArray(entry.checks) || entry.checks.length === 0) {
      problems.push(`${id}: needs at least one check`);
    }
    for (const check of entry.checks ?? []) {
      checkProblems(entry, check, seenCheckIds, problems, root);
      if (entry.group === 'performance' || (check.budget ?? []).length > 0) {
        performanceProblems(entry, check, problems);
      }
    }
    if (id === '29' && (entry.checks ?? []).some((c) => c.status !== 'external')) {
      problems.push('29: every check on criterion 29 is external — it is not verified here');
    }
  }

  for (let n = 1; n <= 67; n += 1) {
    if (!integers.has(n)) problems.push(`criterion ${String(n)} is missing from the registry`);
  }
  for (const [n, ids] of Object.entries(LETTERED)) {
    for (const id of ids)
      if (!seenIds.has(id)) problems.push(`criterion ${n} is missing its part ${id}`);
  }
  problems.push(...validatePhase2Complete(registry));
  problems.push(...validatePhase3Complete(registry));
  problems.push(...validatePhase4Complete(registry));
  return problems;
}
