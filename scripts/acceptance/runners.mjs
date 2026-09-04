/**
 * One result shape from three runners plus the static gates, so a grep over a built bundle
 * joins to a criterion exactly like a unit test does.
 *
 * @typedef {{ id: string, status: 'passed'|'failed'|'skipped', runner: string, tags: string[] }} TestResult
 */
import { tagsIn } from './tags.mjs';

const LIBTEST = /^test\s+(\S+)\s+\.\.\.\s+(ok|FAILED|ignored)\b/u;

/**
 * Cargo's lines arrive wrapped in SGR escapes whenever `CARGO_TERM_COLOR=always`, which
 * `Swatinem/rust-cache` sets on every runner that uses it. Uncoloured locally, coloured on CI —
 * so `RUNNING` matched here and never there, `binary` stayed null, and every integration test was
 * recorded under its bare function name. The harness then reported eight criteria as *did not run*
 * beside seven results as *no check claims it*: the same tests, seen from either side.
 *
 * Stripping is done on the parser side rather than by pinning the environment, because the
 * environment is set by an action this repository does not own.
 */
const SGR = /\u001B\[[0-9;]*m/gu;
/**
 * Cargo's own line above each binary's output. libtest prints a function's **bare** name — an
 * integration test in `core/tests/acceptance_art.rs` reports `ac_56_reroll_offset_is_absolute`
 * and not `acceptance_art::ac_56_…` — so without this the registry could only name functions,
 * two files could not both define `ac_14_a`, and a test moved between files would join anyway.
 * Unit tests inside `core/src` run under `unittests` and already carry their module path, so
 * they are used unchanged.
 */
const RUNNING = /^\s*Running\s+tests\/([A-Za-z0-9_.-]+)\.rs\s/u;
const UNITTESTS = /^\s*Running\s+unittests\s/u;

function result(id, status, runner) {
  return { id, status, runner, tags: tagsIn(id) };
}

/** @returns {TestResult[]} */
export function parseLibtest(stdout) {
  const out = [];
  let binary = null;
  let sawRunning = false;
  let unqualified = 0;
  for (const raw of String(stdout).split('\n')) {
    const line = raw.replace(SGR, '');
    const running = RUNNING.exec(line);
    if (running !== null) {
      binary = running[1];
      sawRunning = true;
      continue;
    }
    if (UNITTESTS.test(line)) {
      binary = null;
      sawRunning = true;
      continue;
    }
    const m = LIBTEST.exec(line.trim());
    if (m === null) continue;
    const status = m[2] === 'ok' ? 'passed' : m[2] === 'FAILED' ? 'failed' : 'skipped';
    if (binary === null && !m[1].includes('::')) unqualified += 1;
    const name = binary === null || m[1].includes('::') ? m[1] : `${binary}::${m[1]}`;
    out.push(result(name, status, 'cargo'));
  }
  // Cargo prints `Running …` to **stderr** and libtest prints `test <bare name> ... ok` to stdout,
  // so a capture that takes stdout alone holds every result and no binary to attribute it to. The
  // harness then reports each integration test twice — once as a criterion that "did not run" and
  // once as a result "no check claims" — which reads as a suite that was never built. Say the
  // actual cause instead: the run happened, the redirection dropped half of it.
  if (unqualified > 0 && !sawRunning) {
    throw new Error(
      `cargo output has ${unqualified} unqualified test names and no "Running" line: ` +
        'cargo writes those to stderr, so this capture dropped them. Pipe with `2>&1`.',
    );
  }
  return out;
}

/** @returns {TestResult[]} */
export function parseVitest(report) {
  const out = [];
  for (const file of report.testResults ?? []) {
    for (const assertion of file.assertionResults ?? []) {
      const status =
        assertion.status === 'passed'
          ? 'passed'
          : assertion.status === 'failed'
            ? 'failed'
            : 'skipped';
      out.push(result(assertion.fullName, status, 'vitest'));
    }
  }
  return out;
}

/** @returns {TestResult[]} */
export function parsePlaywright(report) {
  const out = [];
  const walk = (suite) => {
    for (const spec of suite.specs ?? []) {
      const runs = (spec.tests ?? []).flatMap((t) => t.results ?? []);
      const skipped =
        (spec.tests ?? []).length === 0 ||
        (runs.length > 0 && runs.every((r) => r.status === 'skipped'));
      const status = skipped ? 'skipped' : spec.ok === true ? 'passed' : 'failed';
      out.push(result(spec.title, status, 'e2e'));
    }
    for (const child of suite.suites ?? []) walk(child);
  };
  for (const suite of report.suites ?? []) walk(suite);
  return out;
}

/**
 * A static gate writes `[{ id, status, detail }]`. The id is the check id from the registry,
 * so these join without a tag.
 *
 * @returns {TestResult[]}
 */
export function parseScriptResults(rows) {
  if (!Array.isArray(rows)) throw new TypeError('script results must be an array');
  return rows.map((row) => {
    if (typeof row.id !== 'string' || row.id.length === 0) {
      throw new TypeError('script result has no id');
    }
    if (!['passed', 'failed', 'skipped'].includes(row.status)) {
      throw new TypeError(`script result ${row.id} has an unknown status`);
    }
    return { id: row.id, status: row.status, runner: 'script', tags: [] };
  });
}
