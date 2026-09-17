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
export const RUNNERS = ['cargo', 'vitest', 'e2e', 'script', 'perf', 'manual', 'none'];

const CHECK_ID_P1 = /^AC-(\d{1,2}[a-c]?)(-[a-z0-9]+)*$/u;
// `AC-P2-<section>-<n>`. The slug segment must begin with a letter, or `AC-P2-25-10-ddl` is
// ambiguous with a criterion `P2-25-1` carrying a slug `0-ddl`.
const CHECK_ID_P2 = /^AC-(P2-2[0-5]-\d{1,2})(-[a-z][a-z0-9]*)*$/u;
const P2_ID = /^P2-(2[0-5])-(\d{1,2})$/u;
// R44: a plan number, optionally with the letter suffix of a second half — `13`, `13b`, `13c`
// — and now a phase-2 plan id, `p2-20`.
const OWNER = /^(?:p2-)?\d{2}[a-c]?$/u;
const GATE = /^[A-Z][A-Z0-9-]+$/u;
const WEAKEST = ['automated', 'deferred', 'manual', 'unmeasurable', 'external'];
const LETTERED = { 45: ['45a', '45b', '45c'], 48: ['48a', '48b'] };

/**
 * §26.1's table. `2[0-5]` is an assertion rather than a convenience: §26 owns no criterion of
 * its own, so a `P2-26-*` id is refused by the id form itself instead of by a reviewer.
 */
export const PHASE2_SECTIONS = { 20: 13, 21: 17, 22: 13, 23: 12, 24: 23, 25: 26 };

/**
 * A deferral says *what kind of thing has not happened yet*. `plan` is phase 1's meaning and
 * the default, so all 121 phase-1 deferrals are unchanged. `live-observation` is documentation
 * knowledge until it is checked against a live response, and it **refuses a test id** — a test
 * standing in for an observation is exactly the assertion this status exists to refuse.
 */
export const DEFERRALS = ['plan', 'live-observation'];

/**
 * The two ids ruled live-observation, and nothing else. A third cannot be added without a
 * ruling, which is the point: the status exists to be honest about two known gaps, not to
 * become a place to put anything inconvenient.
 */
export const LIVE_OBSERVATION_CHECKS = ['AC-P2-20-13', 'AC-P2-21-3-floor'];

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
  return p2 === null ? null : p2[1];
}

/** Accepts a criterion id or a check id, because the phase is in the id either way. */
export function phaseOf(id) {
  return /^(?:AC-)?P2-/u.test(String(id)) ? 2 : 1;
}

function sectionOf(criterionId) {
  const m = P2_ID.exec(String(criterionId));
  return m === null ? null : m[1];
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
    problems.push(`${where}: deferred needs an owner plan number, e.g. "13", "13c" or "p2-20"`);
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
    if (!['cargo', 'vitest', 'e2e', 'script'].includes(check.runner)) {
      problems.push(
        `${where}: an automated check runs on cargo, vitest, e2e or script — never perf`,
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
export function validatePhase2Complete(registry) {
  const problems = [];
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
    const section = phaseOf(id) === 2 ? sectionOf(id) : '16';
    if (phaseOf(id) === 2) {
      if (section === null) problems.push(`${id}: a phase-2 criterion id is P2-<20..25>-<n>`);
    } else {
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
    }
    if (!GROUPS.includes(entry.group)) {
      problems.push(`${id}: group is not one of ${GROUPS.join(', ')}`);
    }
    if (typeof entry.title !== 'string' || entry.title.length === 0) problems.push(`${id}: title`);
    const cites = `§${section ?? '16'}.`;
    if (typeof entry.spec !== 'string' || !entry.spec.startsWith(cites)) {
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
  return problems;
}
