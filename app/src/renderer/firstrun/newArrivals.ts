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

export function newArrivals<T extends NewArrivalRow>(
  rows: readonly T[],
  firstRunCompletedAt: number | null,
): readonly T[] {
  return rows.filter((row) => isNewArrival(row, firstRunCompletedAt));
}

/** §8.3's flag, the one the dismissible row sets the query to. */
export const NEW_ARRIVALS_QUERY = 'is:new';

export type AcknowledgingEvent = 'openedProjectPage' | 'launched' | 'peeked';

/**
 * §10.5a: opening the project page, or launching. **Not** Peek — §8.4's Peek is the triage
 * mechanism, reading a card is not acknowledging it, and clearing on Peek lets one arrow-key
 * run down a column erase the whole batch.
 *
 * This classifies the event; writing `acknowledged_at` belongs to whoever owns opening a
 * project page and launching one.
 */
export function acknowledges(event: AcknowledgingEvent): boolean {
  return event === 'openedProjectPage' || event === 'launched';
}

const WEEKDAYS = [
  'Sunday',
  'Monday',
  'Tuesday',
  'Wednesday',
  'Thursday',
  'Friday',
  'Saturday',
] as const;

const MONTHS = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
] as const;

const MS_PER_DAY = 86_400_000;

/** Local midnight, so the label counts calendar days and not 24-hour blocks. */
function startOfLocalDay(epochSecs: number): number {
  const date = new Date(epochSecs * 1000);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/**
 * The day the batch starts from, named the way a reader would name it.
 *
 * The weekday names are a fixed table rather than `Intl`, so the string does not move with the
 * machine's locale — `3 new · since mardi` on an English shelf is a bug, and a test that passes
 * on one machine and fails on another is worse.
 */
export function sinceLabel(earliestCreatedAtSecs: number, nowSecs: number): string {
  const then = new Date(earliestCreatedAtSecs * 1000);
  const days = Math.round(
    (startOfLocalDay(nowSecs) - startOfLocalDay(earliestCreatedAtSecs)) / MS_PER_DAY,
  );
  if (days <= 0) return 'today';
  if (days === 1) return 'yesterday';
  if (days < 7) return WEEKDAYS[then.getDay()] ?? 'earlier';
  const month = MONTHS[then.getMonth()] ?? '';
  const sameYear = then.getFullYear() === new Date(nowSecs * 1000).getFullYear();
  return sameYear
    ? `${String(then.getDate())} ${month}`
    : `${String(then.getDate())} ${month} ${String(then.getFullYear())}`;
}

export interface NewArrivalsNotice {
  readonly count: number;
  readonly since: string;
  /** The rendered row, e.g. `3 new · since Tuesday`. */
  readonly text: string;
  readonly query: string;
}

/**
 * §10.5a's `3 new · since Tuesday` row, or null where there is nothing to announce.
 *
 * The day is the *earliest* arrival's, so the row spans the whole batch rather than naming
 * whenever the last one happened to land.
 */
export function newArrivalsNotice(
  rows: readonly NewArrivalRow[],
  firstRunCompletedAt: number | null,
  nowSecs: number,
): NewArrivalsNotice | null {
  const arrived = newArrivals(rows, firstRunCompletedAt);
  const earliest = arrived.reduce<number | null>(
    (acc, row) => (acc === null || row.createdAt < acc ? row.createdAt : acc),
    null,
  );
  if (earliest === null) return null;
  const since = sinceLabel(earliest, nowSecs);
  return {
    count: arrived.length,
    since,
    text: `${String(arrived.length)} new · since ${since}`,
    query: NEW_ARRIVALS_QUERY,
  };
}
