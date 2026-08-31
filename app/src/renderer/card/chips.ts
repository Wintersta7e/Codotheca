import type { ProjectRow } from '../../generated/protocol';
import { asOfClause } from '../derive/observation';
// R18: the `NEW` predicate is declared once, in first-run, and imported here. It is never
// re-implemented from the columns — §10.5a warns that keyed on `created_at` alone every project
// is new on day one, and that deriving it from `last_interaction_at` would clear the chip for a
// project the user has never opened. Two copies is two chances to drift onto the wrong column.
import { isNewArrival } from '../firstrun/newArrivals';
import type { TokenName } from '../theme/tokens';

/**
 * §7.7's band 4 in phase 1: status chips only. A chip is the label half of the two-segment
 * shields form — one segment, square corners.
 *
 * `NEW` is outside the four-chip cap and renders last. §7.7 orders the column and says it
 * "sorts last"; §10.5a owns the chip and says it is "prepended, never counted". Both hold when
 * the slice happens first and `NEW` is appended after it: four git flags can never suppress the
 * one chip that says the project was not here yesterday, and the column keeps §7.7's order.
 */
export type StatusChipId = 'interrupted' | 'uncommitted' | 'unpushed' | 'behind' | 'new';

export interface StatusChip {
  readonly id: StatusChipId;
  readonly text: string;
  readonly fillToken: TokenName;
  readonly inkToken: TokenName;
  readonly accessibleName: string;
}

export const CHIP_CAP = 4;

export const CHIP_TYPE = {
  fontPx: 7,
  weight: 700,
  tracking: '.12em',
  padding: '2px 5px',
} as const;

export type ChipRow = Pick<
  ProjectRow,
  // R18: `id` is here only so a `ChipRow` satisfies `NewArrivalRow`; no chip reads it.
  | 'id'
  | 'interruptedOp'
  | 'refstateObservedAt'
  | 'isDirty'
  | 'worktreeObservedAt'
  | 'ahead'
  | 'behind'
  | 'fetchHeadAt'
  | 'createdAt'
  | 'acknowledgedAt'
>;

/** `Uncommitted changes as of 14:02` — the chip text and its observation time (§11.7). */
function named(sentence: string, observedAt: number | null, now: number): string {
  if (observedAt === null) return sentence;
  return `${sentence} ${asOfClause(observedAt, now)}`;
}

export function statusChips(
  row: ChipRow,
  now: number,
  firstRunCompletedAt: number | null,
): readonly StatusChip[] {
  const status: StatusChip[] = [];

  if (row.interruptedOp !== null) {
    status.push({
      id: 'interrupted',
      text: 'INTERRUPTED',
      fillToken: 'interrupt',
      // §8.7 snaps #0b0e11 onto --surface-0 and names this chip's ink as one of the four sites
      // that travel with it.
      inkToken: 'surface-0',
      accessibleName: named(`Interrupted ${row.interruptedOp}`, row.refstateObservedAt, now),
    });
  }

  if (row.isDirty === true) {
    status.push({
      id: 'uncommitted',
      text: 'UNCOMMITTED',
      fillToken: 'sig',
      inkToken: 'sig-ink',
      accessibleName: named('Uncommitted changes', row.worktreeObservedAt, now),
    });
  }

  // `is:unpushed` is answered from `ahead` and is null when ahead is unknown (§8.3).
  if (row.ahead !== null && row.ahead > 0) {
    status.push({
      id: 'unpushed',
      text: 'UNPUSHED',
      fillToken: 'sig',
      inkToken: 'sig-ink',
      accessibleName: named('Unpushed commits', row.refstateObservedAt, now),
    });
  }

  // §3.3 contains no fetch, so the count is only as current as whatever fetch the user last ran.
  // With no fetch recorded the chip is omitted, never `BEHIND 0`.
  if (row.fetchHeadAt !== null && row.behind !== null && row.behind > 0) {
    const clause = asOfClause(row.fetchHeadAt, now);
    status.push({
      id: 'behind',
      text: `BEHIND ${String(row.behind)} · ${clause}`,
      fillToken: 'line-5',
      // §8.7 snaps #e7ebef onto --text-1.
      inkToken: 'text-1',
      accessibleName: `Behind by ${String(row.behind)} ${clause}`,
    });
  }

  const capped = status.slice(0, CHIP_CAP);

  // §10.5a: `created_at > first_run_completed_at AND acknowledged_at IS NULL`, including the
  // branch where the stamp is NULL — first run has not finished, so nothing is new relative to
  // what the user had. R18: that is `isNewArrival`'s to state, not this function's.
  if (!isNewArrival(row, firstRunCompletedAt)) return capped;
  return [
    ...capped,
    {
      id: 'new',
      text: 'NEW',
      // Novelty is a library-wide system fact, not a property of the repository, so it takes the
      // accent and never the card jewel.
      fillToken: 'sig',
      inkToken: 'sig-ink',
      // §11.7 pairs a chip with an observation time; `NEW` has none — it is written by §10.5's
      // acknowledgement, not by an observation.
      accessibleName: 'New',
    },
  ];
}
