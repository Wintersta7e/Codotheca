/**
 * The release mode's rules: what a tagged tree must show before it is published. Pure — the CLI
 * passes it the register, the join, the README's text and `git status --porcelain`, so every rule
 * here is tested without a capture or a git history.
 *
 * `ft` grades the first tag: every check that gates it (`firstTag`) passed or is recorded. `1.0`
 * grades everything else too, and the only checks allowed not to run are the ones it derives and
 * prints.
 */
import { renderRegistryLine } from './report.mjs';

/**
 * Checks exempt from running at the 1.0 tag by ruling, each entry carrying its ruling. Closed, like
 * `LIVE_OBSERVATION_CHECKS`: another needs a ruling, not a field.
 */
export const RELEASE_NOT_RUN = [
  {
    id: 'AC-26-roundtrip',
    ruling:
      '§48.5: the user ruled on 2026-09-02 that the app ships no updater, so the update round trip is kept registered as a tripwire and is not run at the tag.',
  },
  {
    id: 'AC-27-appimage',
    ruling:
      '§48.5: the user ruled on 2026-09-02 that the app ships no updater, so the AppImage update path is kept registered as a tripwire and is not run at the tag.',
  },
  {
    id: 'AC-27-packages',
    ruling:
      '§48.5: the user ruled on 2026-09-02 that the app ships no updater, so the package update path is kept registered as a tripwire and is not run at the tag.',
  },
  {
    id: 'AC-P2-21-3-floor',
    ruling:
      "§48.9: observing the floor means provoking a forge's secondary rate limit, which is abuse-shaped, and the shipped floor bounds the retry either way.",
  },
];

/**
 * The checks a 1.0 run may leave unrun, derived from the register: the `perf` runner (by runner,
 * never by group — a behaviour check in a performance criterion runs anywhere), `external`,
 * `unmeasurable`, and `RELEASE_NOT_RUN`.
 */
export function derivedNotRun(registry) {
  const ruled = new Map(RELEASE_NOT_RUN.map((r) => [r.id, r.ruling]));
  const out = [];
  for (const entry of registry.criteria ?? []) {
    for (const check of entry.checks ?? []) {
      let why = null;
      if (check.runner === 'perf') why = 'the perf runner: measured on the reference machines';
      else if (check.status === 'external') why = 'external';
      else if (check.status === 'unmeasurable') why = 'unmeasurable';
      else if (ruled.has(check.id)) why = ruled.get(check.id);
      if (why !== null) out.push({ id: check.id, why });
    }
  }
  return out;
}

const FIGURE = /(\d[\d,]*)\s+checks\b[^.]{0,120}?\b(\d[\d,]*)\s+(?:written\s+)?criteria\b/gu;

/** Every "<n> checks … <m> (written) criteria" pair a text quotes, as numbers. */
export function readmeRegisterFigures(text) {
  const number = (s) => Number.parseInt(s.replaceAll(',', ''), 10);
  return [...String(text).matchAll(FIGURE)].map((m) => ({
    checks: number(m[1]),
    criteria: number(m[2]),
  }));
}

/**
 * Why a release run refuses this tree, or `null`. `porcelain` is `git status --porcelain`: a dirty
 * tree is refused before anything is graded, because a tag grades what was committed.
 */
export function dirtyTreeRefusal(porcelain) {
  const lines = String(porcelain)
    .split('\n')
    .filter((line) => line.trim().length > 0);
  if (lines.length === 0) return null;
  return (
    `the working tree is dirty (${String(lines.length)} paths, first ${lines[0].trim()}) — a` +
    ' release run grades a committed tree'
  );
}

function readmeProblems(registry, readme) {
  const pairs = readmeRegisterFigures(readme);
  if (pairs.length === 0) {
    return ['README.md quotes no register figure — a comparison of nothing proves nothing'];
  }
  const [, criteria, checks] = /^(\d+) criteria \/ (\d+) checks/u
    .exec(renderRegistryLine(registry))
    .map(Number);
  return pairs
    .filter((p) => p.checks !== checks || p.criteria !== criteria)
    .map(
      (p) =>
        `README.md quotes ${String(p.checks)} checks and ${String(p.criteria)} criteria;` +
        ` the register holds ${String(checks)} and ${String(criteria)}`,
    );
}

/**
 * `joined` is `joinResults`' output, with the record status applied. Returns the derived not-run
 * set, the problems, and `refused` — non-null when the tree is dirty, in which case nothing is
 * graded.
 */
export function releaseProblems(registry, joined, mode, { readme, porcelain }) {
  const refused = dirtyTreeRefusal(porcelain);
  if (refused !== null) return { notRun: [], problems: [], refused };
  const notRun = derivedNotRun(registry);
  const exempt = new Set(notRun.map((n) => n.id));
  const result = new Map(joined.checks.map((c) => [c.id, c]));
  const problems = [];

  for (const entry of registry.criteria ?? []) {
    for (const check of entry.checks ?? []) {
      const got = result.get(check.id);
      const why = got?.recordWhy === undefined ? '' : ` (${got.recordWhy})`;
      const state = `${String(check.status)}, ${String(got?.result ?? 'not-run')}${why}`;
      const done =
        (check.status === 'automated' && got?.result === 'passed') ||
        (check.status === 'manual' && got?.result === 'recorded') ||
        (check.deferral === 'live-observation' && got?.result === 'recorded');
      if (mode === 'ft') {
        if (check.firstTag === true && !done) {
          problems.push(`${check.id}: gates the first tag and is ${state}`);
        }
        continue;
      }
      if (exempt.has(check.id)) continue;
      // R46: once the phase's plans have merged, a deferral to one is a deferral to nobody —
      // whether or not a test under its name happens to pass.
      if (check.status === 'deferred' && (check.deferral ?? 'plan') === 'plan') {
        problems.push(`${check.id}: deferred to plan ${String(check.owner)} at the tag`);
      } else if (!done) {
        problems.push(`${check.id}: is ${state}, and only the derived set may be not run`);
      }
    }
  }
  problems.push(...readmeProblems(registry, readme));
  return { notRun, problems, refused: null };
}
