import type { ProjectRow } from '../../generated/protocol';

/**
 * §10.5a's `NEW` predicate, declared **once** (R18).
 *
 * The rule is `created_at > first_run_completed_at AND acknowledged_at IS NULL`, and every half
 * of it is load-bearing. Keyed on `created_at` alone *every* project is new on day one. Derived
 * from `last_interaction_at` instead of `acknowledged_at` the chip would clear for a project the
 * user has never opened, because that column counts reflog activity performed outside the app —
 * a chip that looks right and is wrong. Two copies of this predicate is two chances to drift
 * onto that column, which is why `statusChips` (§7.7's band 4) imports it rather than restating
 * it from the same columns.
 *
 * The boundary is exclusive: a project indexed *by* first run is not new relative to what the
 * user already had, and a `null` boundary means first run has not finished, so nothing is.
 */
export type NewArrivalRow = Pick<ProjectRow, 'id' | 'createdAt' | 'acknowledgedAt'>;

export function isNewArrival(row: NewArrivalRow, firstRunCompletedAt: number | null): boolean {
  if (firstRunCompletedAt === null) return false;
  if (row.acknowledgedAt !== null) return false;
  return row.createdAt > firstRunCompletedAt;
}
