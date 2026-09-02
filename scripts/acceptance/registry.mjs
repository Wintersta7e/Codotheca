/**
 * The acceptance registry: one entry per §16 criterion, one or more checks per entry.
 *
 * A check's status is the whole point of the file. `automated` means a test runs today and
 * gates the build; `deferred` means the named plan lands the named test; `manual` names a
 * review gate and states why a machine cannot do it; `unmeasurable` means there is no honest
 * event pair or no budget to gate on; `external` is criterion 29 and nothing else.
 */
import { readFileSync } from 'node:fs';

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

const CHECK_ID = /^AC-(\d{1,2}[a-c]?)(-[a-z0-9]+)*$/u;
// R44: a plan number, optionally with the letter suffix of a second half — `13`, `13b`, `13c`.
const OWNER = /^\d{2}[a-c]?$/u;
const GATE = /^[A-Z][A-Z0-9-]+$/u;
const WEAKEST = ['automated', 'deferred', 'manual', 'unmeasurable', 'external'];
const LETTERED = { 45: ['45a', '45b', '45c'], 48: ['48a', '48b'] };

export function criterionOf(checkId) {
  const m = CHECK_ID.exec(String(checkId));
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

function checkProblems(entry, check, seenCheckIds, problems) {
  const where = `${entry.id}/${String(check.id)}`;
  if (!CHECK_ID.test(String(check.id))) {
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
  if (check.status === 'deferred') {
    if (!OWNER.test(String(check.owner))) {
      problems.push(`${where}: deferred needs an owner plan number, e.g. "13" or "13c"`);
    }
    if (typeof check.test !== 'string' || check.test.length === 0) {
      problems.push(`${where}: deferred needs the test id it will carry`);
    }
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

export function validateRegistry(registry) {
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
    const n = Number.parseInt(id, 10);
    if (!Number.isInteger(n) || n < 1 || n > 67) problems.push(`${id}: criterion id outside 1..67`);
    else integers.add(n);
    const letter = id.slice(String(n).length);
    if (letter !== '' && !(LETTERED[n] ?? []).includes(id)) {
      problems.push(`${id}: only 45a/b/c and 48a/b carry a letter`);
    }
    if (letter === '' && LETTERED[n] !== undefined) {
      problems.push(`${id}: criterion ${String(n)} appears only in lettered form`);
    }
    if (!GROUPS.includes(entry.group)) {
      problems.push(`${id}: group is not one of ${GROUPS.join(', ')}`);
    }
    if (typeof entry.title !== 'string' || entry.title.length === 0) problems.push(`${id}: title`);
    if (typeof entry.spec !== 'string' || !entry.spec.startsWith('§16.')) {
      problems.push(`${id}: spec must cite §16.<n>`);
    }
    if (!Array.isArray(entry.checks) || entry.checks.length === 0) {
      problems.push(`${id}: needs at least one check`);
    }
    for (const check of entry.checks ?? []) {
      checkProblems(entry, check, seenCheckIds, problems);
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
  return problems;
}
