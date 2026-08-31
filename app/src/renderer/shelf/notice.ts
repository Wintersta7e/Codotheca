/** §8.0's table, one kind per row. `coreFailure` covers all three of priority 1's causes —
 *  `core.degraded`, `core.error` and the `GIT_MISSING` / `GIT_TOO_OLD` floor — because they
 *  share a priority, a border and the absence of a dismissal.
 *
 *  The names are the ones §1.9 stores: the dismissal key is `notice.dismissed.` + the kind and
 *  nothing translates in between, so `identity` is spelled the way §8.0 row 4a and
 *  §1.4 spell it in the column itself. */
export type NoticeKind =
  | 'coreFailure'
  | 'scanResumed'
  | 'problems'
  | 'targetUnresolved'
  | 'identity'
  | 'residency'
  | 'newArrivals';

/** Index is the priority. `identity` is §8.0's 4a and sits between 4 and 5. */
export const NOTICE_PRIORITY: readonly NoticeKind[] = [
  'coreFailure',
  'scanResumed',
  'problems',
  'targetUnresolved',
  'identity',
  'residency',
  'newArrivals',
];

/** Priority 1 stands until the condition clears; every other row has a dismissal, whether it is
 *  written by the DISMISS control or by answering the ask. */
export const UNDISMISSABLE: readonly NoticeKind[] = ['coreFailure'];

export interface NoticeAction {
  readonly label: string;
  readonly kind: 'primary' | 'secondary';
  readonly run: () => void;
}

/** The copy is the owning section's (§10.5, §11.1, §11.2, §11.3, §11.5, §1.4). This module
 *  decides only which one renders and where its dismissal is recorded. */
export interface Notice {
  readonly kind: NoticeKind;
  readonly scope: string | null;
  readonly title: string;
  readonly body: string;
  readonly actions: readonly NoticeAction[];
}

export function noticeAccent(kind: NoticeKind): 'fail-hot' | 'sig' {
  return kind === 'coreFailure' ? 'fail-hot' : 'sig';
}

export function noticeIsDismissible(kind: NoticeKind): boolean {
  return !UNDISMISSABLE.includes(kind);
}

/** `notice.dismissed.<kind>[:<scope>]`, written into `view_state` (§1.9). */
export function noticeDismissKey(kind: NoticeKind, scope: string | null): string {
  return scope === null ? `notice.dismissed.${kind}` : `notice.dismissed.${kind}:${scope}`;
}

export function selectNotice(
  candidates: readonly Notice[],
  dismissed: readonly string[],
): Notice | null {
  for (const kind of NOTICE_PRIORITY) {
    const candidate = candidates.find((entry) => entry.kind === kind);
    if (candidate === undefined) continue;
    if (!noticeIsDismissible(kind)) return candidate;
    if (dismissed.includes(noticeDismissKey(kind, candidate.scope))) continue;
    return candidate;
  }
  return null;
}
