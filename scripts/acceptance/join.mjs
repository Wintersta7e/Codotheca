/**
 * Join runner results to registry checks, then diff the failing set against the baseline.
 *
 * The diff is over identities, never counts: a suite with a known-red baseline hides
 * regressions behind a plausible total.
 */
// R44 — second halves own criteria too, and a phase-2 plan id is `p2-20`.
const OWNER = /^(?:p2-)?\d{2}[a-c]?$/u;

export function validateBaseline(baseline, registry) {
  const problems = [];
  const known = new Set(registry.criteria.map((c) => c.id));
  const seen = new Set();
  for (const row of baseline.knownRed ?? []) {
    const where = String(row.test ?? '<no test>');
    if (typeof row.test !== 'string' || row.test.length === 0) {
      problems.push('a baseline entry has no test id');
    }
    if (seen.has(row.test)) problems.push(`${where}: duplicate baseline entry`);
    seen.add(row.test);
    if (!known.has(String(row.criterion))) {
      problems.push(`${where}: criterion ${String(row.criterion)} is not in the registry`);
    }
    if (!OWNER.test(String(row.owner))) problems.push(`${where}: needs an owner plan number`);
    if (typeof row.reason !== 'string' || row.reason.length < 20) {
      problems.push(`${where}: needs a reason`);
    }
  }
  return problems;
}

export function joinResults(registry, results) {
  const byId = new Map(results.map((r) => [r.id, r]));
  const claimed = new Set();
  const checks = [];
  for (const entry of registry.criteria) {
    for (const check of entry.checks) {
      const found = check.test === undefined ? undefined : byId.get(check.test);
      if (found !== undefined) claimed.add(found.id);
      checks.push({
        id: check.id,
        criterion: entry.id,
        status: check.status,
        runner: check.runner,
        owner: check.owner ?? null,
        gate: check.gate ?? null,
        test: check.test ?? null,
        result: found === undefined ? 'not-run' : found.status,
      });
    }
  }
  const untagged = results.filter((r) => !claimed.has(r.id) && r.tags.length > 0);
  return { checks, untagged };
}

export function diffAgainstBaseline(results, baseline) {
  const failing = new Set(results.filter((r) => r.status === 'failed').map((r) => r.id));
  const ran = new Set(results.map((r) => r.id));
  const known = new Set((baseline.knownRed ?? []).map((r) => r.test));

  return {
    newFailures: [...failing].filter((id) => !known.has(id)).sort(),
    stale: [...known].filter((id) => ran.has(id) && !failing.has(id)).sort(),
    missing: [...known].filter((id) => !ran.has(id)).sort(),
  };
}

/**
 * The suites a run actually executed. A job that ran only the Rust suite must not report every
 * shell-side check as missing, so a runner absent from the results is reported as absent rather
 * than as a wall of failures.
 */
export function runnersPresent(results) {
  return new Set(results.map((r) => r.runner));
}

/**
 * The runners an automated check needs and this run did not execute. This is deliberately a
 * separate list rather than silence: it is the difference between "the test is gone" and "the
 * suite was not run here", and a reader who cannot see which one it was will assume the
 * comfortable one.
 */
export function absentRunners(join, results) {
  const present = runnersPresent(results);
  const wanted = new Set(join.checks.filter((c) => c.status === 'automated').map((c) => c.runner));
  return [...wanted].filter((r) => !present.has(r)).sort();
}

/**
 * `results` is optional only so a unit test can gate a join in isolation. Pass it from any real
 * run: without it every absent suite reads as a deleted test.
 */
export function gateProblems(join, diff, results = null) {
  const problems = [];
  const absent = results === null ? new Set() : new Set(absentRunners(join, results));
  for (const check of join.checks) {
    if (check.status !== 'automated') continue;
    if (absent.has(check.runner)) continue;
    if (check.result === 'not-run') {
      problems.push(`${check.id}: marked automated but its test ${String(check.test)} did not run`);
      continue;
    }
    if (check.result === 'skipped') {
      // A skip is not a pass. The one spec in this repository that renders a real frame skips
      // itself when the release core is absent, and a criterion that reads as covered on a
      // machine that never ran it is the gate-that-cannot-fail shape one level down.
      problems.push(
        `${check.id}: marked automated and its test ${String(check.test)} skipped — a skip is not a pass`,
      );
    }
  }
  for (const id of diff.newFailures) problems.push(`new failure, not in the baseline: ${id}`);
  for (const id of diff.stale)
    problems.push(`stale baseline entry — it passes now, delete it: ${id}`);
  for (const id of diff.missing) problems.push(`baseline names a test that did not run: ${id}`);
  for (const row of join.untagged) {
    problems.push(`test carries a criterion tag but no check claims it: ${row.id}`);
  }
  return problems;
}
