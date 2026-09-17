/**
 * §11.1's scan summary, in strings. The group order is plan 17's `GROUP_ORDER` and is not
 * restated; this module supplies the words the core deliberately does not.
 */
import type { ProblemItem, ProblemKind, Problems } from '../../generated/protocol';

export const PROBLEM_GROUP_LABEL: Readonly<Record<ProblemKind, string>> = {
  permission_denied: 'PERMISSION DENIED',
  untrusted_repo: 'UNTRUSTED REPOSITORIES',
  unreadable_repo: 'UNREADABLE REPOSITORIES',
  deferred_slow: 'DEFERRED-SLOW',
  clock_skew: 'CLOCK SKEW',
  non_utf8_path: 'NON-UTF-8 PATHS',
  offline_store: 'OFFLINE STORES',
  ambiguous_lineage: 'AMBIGUOUS LINEAGE',
  // [p2] §24.3c: a staging directory an install left behind and the sweep could not warrant.
  // The record is `Readonly<Record<ProblemKind, string>>`, so the ninth label is a type error
  // until it is added — which is the point.
  abandoned_install: 'ABANDONED INSTALLS',
};

export interface HeaderClause {
  readonly figure: string;
  readonly label: string;
}

/** Fixed locale: §11.1's `214,903` is the shape, not the reader's regional preference. */
function figure(n: number): string {
  return n.toLocaleString('en-US');
}

function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many;
}

/**
 * `null` when no scan has ever run. A run with `walkedDirs = 0` has not been measured, and a
 * zeroed header would be exactly the claim this screen exists to avoid making.
 */
/**
 * Dismissing the banner clears this run's problems from the summary too — ruled by the owner,
 * because a list that survives its own dismissal reads as a control that did nothing.
 *
 * It is a heading of its own rather than the "nothing went wrong" one: the problems happened and
 * saying otherwise would be inventing a clean scan. What changed is that they were acknowledged.
 */
export const PROBLEMS_DISMISSED_HEADING = 'THIS SCAN’S PROBLEMS WERE DISMISSED';

export function summaryHeaderClauses(
  problems: Problems,
  /** When the run's banner was dismissed, the problem clause goes with the list it counted. */
  problemsDismissed = false,
): readonly HeaderClause[] | null {
  if (problems.runId === null) return null;
  const h = problems.header;
  const clauses: HeaderClause[] = [
    { figure: figure(h.walkedDirs), label: 'directories walked' },
    { figure: figure(h.repositories), label: plural(h.repositories, 'repository', 'repositories') },
  ];
  // Omitted while the scan is in flight (`null`); rendered at a measured zero. And omitted once
  // dismissed — a count standing over an empty list is the same contradiction one line up.
  if (h.problemCount !== null && !problemsDismissed) {
    clauses.push({
      figure: figure(h.problemCount),
      label: plural(h.problemCount, 'problem', 'problems'),
    });
  }
  // §11.1 removes this clause entirely at zero: nothing broke, so nothing is reported.
  if (h.ambiguousLineageCount !== null && h.ambiguousLineageCount > 0) {
    clauses.push({
      figure: figure(h.ambiguousLineageCount),
      label: 'with ambiguous lineage',
    });
  }
  return clauses;
}

/** The same line as a plain string. §10.2's scan line renders this and clicks through here. */
export function summaryHeaderLine(problems: Problems): string | null {
  const clauses = summaryHeaderClauses(problems);
  if (clauses === null) return null;
  return clauses.map((c) => `${c.figure} ${c.label}`).join(' · ');
}

export const NO_SCAN_YET_HEADING = 'NO SCAN HAS RUN YET';
export const NO_SCAN_YET_BODY =
  'Nothing has been walked, so there is nothing to report. Add a folder and run a scan.';

export const NO_PROBLEMS_HEADING = 'NOTHING WENT WRONG';

/** §11.1, verbatim. The note idiom: `border-left: 2px solid var(--line-5)`, body 11.5px/1.45. */
export const AMBIGUOUS_GROUP_NOTE =
  'These repositories have no remote, and their history matches more than one project you ' +
  'already have. Codotheca did not guess which. Each one is indexed as its own project and ' +
  'works normally — only the link to the others is missing. Joining two projects cannot be ' +
  'undone yet, so it is not offered here.';

/** The group's one control. It navigates to §8.5 and merges nothing. */
export const OPEN_PROJECT_LABEL = 'OPEN PROJECT';

/**
 * §11.1's two forms. The core sends at most `AMBIGUOUS_NAMES_SHOWN` names and the full id list,
 * so the tail count is the ids' length minus the names shown — never a stored candidate set,
 * which §1.1 forbids because it goes stale as J4 progresses.
 */
export function candidateLine(item: ProblemItem): string | null {
  const names = item.candidateNames;
  const first = names[0];
  const second = names[1];
  if (first === undefined || second === undefined) return null;
  const rest = item.candidateProjectIds.length - names.length;
  return rest > 0
    ? `Same history as ${first}, ${second} and ${rest} more.`
    : `Same history as ${first} and ${second}.`;
}

/**
 * What renders beneath a row's `path_display`.
 *
 * `offline_store` renders nothing: criterion 63 forbids a drive or volume name on an offline
 * row, and `detail` is a core-authored string this renderer cannot vet for one. The wire's
 * `lastSeenAt` would carry §11.1's `last seen <age>`, but the core sources this group from
 * `scan_problem`, which has no `location_id` and so no observation clock — the field is `null`
 * for every row in it, and an age drawn from nothing is unknown rendered as a fact.
 */
export function summaryDetailLine(kind: ProblemKind, item: ProblemItem): string | null {
  if (kind === 'ambiguous_lineage') return candidateLine(item);
  if (kind === 'offline_store') return null;
  return item.detail;
}
