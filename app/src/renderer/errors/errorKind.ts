/**
 * §11.1's per-project error state, and the one place a core error code becomes prose.
 *
 * §2.4: the core's `message` is diagnostic and is never shown. Every string in this module is
 * keyed off the closed `ErrorCode` union generated from `protocol/schema/protocol.json`, so a
 * code added there fails `npm run typecheck` at the `never` assignment below rather than
 * reaching a user as a blank badge or a stack trace.
 *
 * §1.2's **never-succeeded** state is declared here too, once, for every surface that reads it.
 * It is distinct from stale-but-once-known, and neither may be rendered as the other: this one
 * says *why nothing is known*, the other says *as of when*. Never-succeeded is `error_kind`
 * non-NULL **with no observation ever recorded**; a project that has an observation and a later
 * error is stale, not unread.
 *
 * Two copies of that predicate would let one project read as *never indexed* to the badge and
 * *indexed* to the note in the same frame, which is why it is stated here and imported.
 */
import type { ErrorCode, LocationId, ProjectId, ProjectRow } from '../../generated/protocol';
import { formatAge } from '../derive/observation';

export type NeverSucceededRow = Pick<
  ProjectRow,
  'errorKind' | 'refstateObservedAt' | 'worktreeObservedAt'
>;

export function neverSucceeded(row: NeverSucceededRow): boolean {
  if (row.errorKind === null) return false;
  return row.refstateObservedAt === null && row.worktreeObservedAt === null;
}

export type ErrorPlacement = 'project' | 'startup' | 'never';
export type ErrorActionKind = 'trust' | 'relocate';

/** §11.1's two drawn controls. `locations.setTrusted` and the shell's dialog back them. */
export const ERROR_ACTION_LABEL: Readonly<Record<ErrorActionKind, string>> = {
  trust: 'TRUST THIS REPOSITORY',
  relocate: 'RELOCATE',
};

export const TRY_AGAIN_LABEL = 'TRY AGAIN';

export interface ExplainedError {
  readonly placement: ErrorPlacement;
  readonly badge: string | null;
  readonly prose: string | null;
  readonly action: ErrorActionKind | null;
}

/** Sets on every project at once, so §11.1 renders it once in §11.2's startup surface. */
const AT_STARTUP: ExplainedError = { placement: 'startup', badge: null, prose: null, action: null };
/** App-level: §11.1 states these never reach `project.error_kind`. */
const NOT_A_PROJECT_STATE: ExplainedError = {
  placement: 'never',
  badge: null,
  prose: null,
  action: null,
};

export function explainErrorKind(kind: ErrorCode): ExplainedError {
  switch (kind) {
    case 'PERMISSION_DENIED':
      return {
        placement: 'project',
        badge: 'NOT INDEXED',
        prose: "Can't read this folder. Nothing is lost — it just isn't indexed.",
        action: null,
      };
    case 'UNTRUSTED_REPO':
      return {
        placement: 'project',
        badge: 'NOT TRUSTED',
        prose: "Git won't open a repository owned by another user.",
        action: 'trust',
      };
    case 'REPO_UNREADABLE':
      return {
        placement: 'project',
        badge: 'NOT INDEXED',
        prose: "This looks like a repository, but git can't open it.",
        action: null,
      };
    case 'PATH_GONE':
      return {
        placement: 'project',
        badge: 'MISSING',
        prose: "The folder isn't where it was.",
        action: 'relocate',
      };
    case 'STORE_OFFLINE':
      // §8.5.2's `offline` presence row keeps the design's frozen-condition sentence; this is
      // the never-succeeded state, which has no condition to freeze (§1.2, §5.4a).
      return {
        placement: 'project',
        badge: 'OFFLINE',
        prose:
          'The drive holding this folder is not mounted, so this repository has never been read.',
        action: null,
      };
    case 'BUDGET_EXCEEDED':
      // Only when no job has ever succeeded; otherwise it is `deferred_slow`, not an error.
      return {
        placement: 'project',
        badge: 'NOT INDEXED',
        prose: 'Slow to read, so it went to the back of the queue.',
        action: null,
      };
    case 'GIT_MISSING':
    case 'GIT_TOO_OLD':
      return AT_STARTUP;
    case 'CORE_RESTARTED':
    case 'PROTOCOL':
    case 'INTERNAL':
    case 'PROJECT_MERGED':
      return NOT_A_PROJECT_STATE;
    default: {
      const unhandled: never = kind;
      return unhandled;
    }
  }
}

/** §11.1's `LAST TRIED <time>`. NULL `error_at` renders no line, never a zero and never a date. */
export function lastTriedLine(errorAtSecs: number | null, nowSecs: number): string | null {
  if (errorAtSecs === null) return null;
  const age = formatAge(nowSecs - errorAtSecs);
  return age === 'just now' ? 'LAST TRIED JUST NOW' : `LAST TRIED ${age} AGO`;
}

/** The six fields the block reads, so a caller passes a row and a test passes six values. */
export interface ErrorSubject {
  readonly projectId: ProjectId;
  readonly locationId: LocationId | null;
  readonly errorKind: ErrorCode | null;
  readonly errorAt: number | null;
  readonly refstateObservedAt: number | null;
  readonly worktreeObservedAt: number | null;
}

export function errorSubject(row: ProjectRow): ErrorSubject {
  return {
    projectId: row.id,
    locationId: row.primaryLocation?.id ?? null,
    errorKind: row.errorKind,
    errorAt: row.errorAt,
    refstateObservedAt: row.refstateObservedAt,
    worktreeObservedAt: row.worktreeObservedAt,
  };
}
