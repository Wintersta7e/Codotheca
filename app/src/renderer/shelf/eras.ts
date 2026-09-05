import { formatTrackedBytes } from '../format/size.js';
import type { ShelfRow } from './row.js';

export const ERA_LIVE_DAYS = 7;
export const ERA_MONTH_DAYS = 30;
export const ERA_QUARTER_DAYS = 90;
/** Named year sections run from the cut year down to cut − 10; the tail is cut − 11 and earlier. */
export const ERA_NAMED_YEARS = 10;
/** §8.1: not the design's 60. 60 is ten rows at a 303 px pitch — under three screenfuls. */
export const ERA_COLLAPSE_THRESHOLD = 150;
/** Everything older than EARLIER THIS YEAR, i.e. every year section and the three tails. */
export const ERA_COLLAPSE_MIN_ORDER = 10;

const DAY = 86_400;

export function eraSectionIdFor(row: ShelfRow, now: number): string {
  // §23.4: tested **first**, before both overrides — R13's mirror of `era_section_id_for`.
  // Order 98 alone does not achieve it: `isArchived` is a user flag, a user may archive a
  // not-cloned project, and `era:archived` is an interleaved section whose header sums tracked
  // bytes. §23.1: `primaryLocation IS NULL` is the whole predicate; `presence IS NULL` is the
  // same predicate rendered, not a second source.
  if (row.primaryLocation === null) return 'era:notcloned';
  if (row.isArchived) return 'era:archived';
  if (row.isSubmodule) return 'era:submodules';

  const ageDays = (now - row.lastTouchedAt) / DAY;
  if (ageDays <= ERA_LIVE_DAYS) return 'era:live';
  if (ageDays <= ERA_MONTH_DAYS) return 'era:month';
  if (ageDays <= ERA_QUARTER_DAYS) return 'era:q';

  const cutYear = new Date(now * 1000).getFullYear();
  const touchedYear = new Date(row.lastTouchedAt * 1000).getFullYear();
  if (touchedYear >= cutYear) return 'era:year';
  if (touchedYear >= cutYear - ERA_NAMED_YEARS) return `era:${String(touchedYear)}`;
  return 'era:tail';
}

const FIXED_ORDER: Readonly<Record<string, number>> = {
  'era:live': 0,
  'era:month': 1,
  'era:q': 2,
  'era:year': 3,
  'era:tail': 90,
  'era:archived': 92,
  'era:submodules': 94,
  'era:notcloned': 98,
};

export function eraSectionOrder(id: string, cutAgainstYear: number): number {
  const fixed = FIXED_ORDER[id];
  if (fixed !== undefined) return fixed;
  const year = Number(id.slice('era:'.length));
  return 10 + (cutAgainstYear - year);
}

const FIXED_LABEL: Readonly<Record<string, string>> = {
  'era:live': 'LIVE',
  'era:month': 'THIS MONTH',
  'era:q': 'THIS QUARTER',
  'era:year': 'EARLIER THIS YEAR',
  'era:archived': 'ARCHIVED',
  'era:submodules': 'SUBMODULES',
  // §8.1's table left this one blank and §23.4 owns it. Without the entry the fallback below
  // prints the header as lowercase `notcloned`.
  'era:notcloned': 'NOT CLONED',
};

/** §8.1: labels are shell-owned prose; the core emits the id, the order and the cut year. */
export function eraSectionLabel(id: string, cutAgainstYear: number): string {
  if (id === 'era:tail') return `${String(cutAgainstYear - ERA_NAMED_YEARS - 1)} AND EARLIER`;
  return FIXED_LABEL[id] ?? id.slice('era:'.length);
}

export interface SectionAggregate {
  readonly count: number;
  readonly trackedBytes: number;
  /** Projects whose tracked inventory has completed — never the section count. */
  readonly indexedCount: number;
  readonly unpushed: number;
  readonly uncommitted: number;
  readonly interrupted: number;
  /** Projects with no J1 result. An absent flag line may only mean *observed, nothing to report*. */
  readonly unchecked: number;
}

export function aggregateSection(rows: readonly ShelfRow[]): SectionAggregate {
  let trackedBytes = 0;
  let indexedCount = 0;
  let unpushed = 0;
  let uncommitted = 0;
  let interrupted = 0;
  let unchecked = 0;
  for (const row of rows) {
    if (row.sizeTrackedBytes !== null) {
      trackedBytes += row.sizeTrackedBytes;
      indexedCount += 1;
    }
    if ((row.ahead ?? 0) > 0) unpushed += 1;
    if (row.isDirty === true) uncommitted += 1;
    if (row.interruptedOp !== null) interrupted += 1;
    if (row.refstateObservedAt === null) unchecked += 1;
  }
  return {
    count: rows.length,
    trackedBytes,
    indexedCount,
    unpushed,
    uncommitted,
    interrupted,
    unchecked,
  };
}

// R12: one `formatTrackedBytes` in the product. Its body — one decimal of GB at ≥ 0.1 GB, whole
// MB below, both divisors binary — is §8.1 written out and lives in `format/size.ts`, which also
// carries the mutation test that fails when the divisor becomes 1000. The re-export keeps
// `./eras.js` the import path this module's own callers already use.
export { formatTrackedBytes };

export function summaryText(agg: SectionAggregate): string {
  const plural = agg.count === 1 ? 'project' : 'projects';
  const coverage = agg.indexedCount < agg.count ? ` (of ${String(agg.indexedCount)} indexed)` : '';
  return `${String(agg.count)} ${plural} · ${formatTrackedBytes(agg.trackedBytes)} tracked${coverage}`;
}

/** The one truncation step: the byte aggregate and its parenthetical go as a unit. */
export function summaryTextTruncated(agg: SectionAggregate): string {
  return `${String(agg.count)} ${agg.count === 1 ? 'project' : 'projects'}`;
}

export function flagLineText(agg: SectionAggregate): string | null {
  const parts: string[] = [];
  if (agg.unpushed > 0) parts.push(`${String(agg.unpushed)} unpushed`);
  if (agg.uncommitted > 0) parts.push(`${String(agg.uncommitted)} uncommitted`);
  if (agg.interrupted > 0) parts.push(`${String(agg.interrupted)} interrupted`);
  if (agg.unchecked > 0) parts.push(`${String(agg.unchecked)} unchecked`);
  return parts.length === 0 ? null : parts.join(' · ');
}
