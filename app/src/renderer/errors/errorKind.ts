/**
 * §1.2's **never-succeeded** state, declared once for every surface that reads it.
 *
 * It is distinct from stale-but-once-known, and neither may be rendered as the other: this one
 * says *why nothing is known*, the other says *as of when*. Never-succeeded is `error_kind`
 * non-NULL **with no observation ever recorded**; a project that has an observation and a later
 * error is stale, not unread.
 *
 * Two copies of this predicate would let one project read as *never indexed* to the badge and
 * *indexed* to the note in the same frame, which is why it is stated here and imported.
 */
import type { ProjectRow } from '../../generated/protocol';

export type NeverSucceededRow = Pick<
  ProjectRow,
  'errorKind' | 'refstateObservedAt' | 'worktreeObservedAt'
>;

export function neverSucceeded(row: NeverSucceededRow): boolean {
  if (row.errorKind === null) return false;
  return row.refstateObservedAt === null && row.worktreeObservedAt === null;
}
