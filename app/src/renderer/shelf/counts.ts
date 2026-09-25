import { parseQuery } from '../../shared/query/parse.js';
import type { QueryContext } from './evaluate.js';
import { evaluateQuery } from './evaluate.js';
import type { ShelfRow } from './row.js';

/** §8.0b. The accents are §8.7 tokens except COLD's, which §5.4a owns as `abandoned`'s fill. */
export const ATTENTION_CHIPS = [
  { id: 'all', label: 'ALL', sub: 'THE WHOLE LIBRARY', query: '', accent: 'var(--text-2)' },
  {
    id: 'unpushed',
    label: 'UNPUSHED',
    sub: 'WORK ONLY ON THIS DISK',
    query: 'is:unpushed',
    accent: 'var(--sig)',
  },
  {
    id: 'uncommitted',
    label: 'UNCOMMITTED',
    sub: 'CHANGES NOT YET IN GIT',
    query: 'is:dirty',
    accent: 'var(--sig)',
  },
  {
    id: 'cold',
    label: 'COLD',
    sub: 'NOT TOUCHED IN OVER A YEAR',
    query: 'touched:>365d',
    accent: '#8a6a4a',
  },
] as const;

export interface ShelfCounts {
  readonly matched: number;
  readonly total: number;
  readonly reference: number;
  readonly classified: number;
  readonly classificationKnown: boolean;
}

export function shelfCounts(rows: readonly ShelfRow[], matchedCount: number): ShelfCounts {
  let total = 0;
  let reference = 0;
  let classified = 0;
  let classificationKnown = false;
  for (const row of rows) {
    if (row.isReference) reference += 1;
    else if (!row.isHidden) total += 1;
    if (row.authoredByUser !== null) {
      classificationKnown = true;
      classified += 1;
    }
  }
  return { matched: matchedCount, total, reference, classified, classificationKnown };
}

/**
 * §8.0b, §10.4: the exclusion count is qualified wherever `authored_by_user` is still NULL —
 * an unqualified figure claims a classification pass that has not finished.
 */
export function headlineText(counts: ShelfCounts, sortLabel: string): string {
  const complete =
    counts.classificationKnown && counts.classified === counts.total + counts.reference;
  const qualifier = complete ? '' : ` (OF ${String(counts.classified)} CLASSIFIED)`;
  return `${String(counts.matched)} OF ${String(counts.total)} · ${sortLabel} · ${String(counts.reference)} REFERENCE EXCLUDED${qualifier}`;
}

export function attentionCounts(
  rows: readonly ShelfRow[],
  ctx: QueryContext,
): Readonly<Record<string, number>> {
  const out: Record<string, number> = {};
  for (const chip of ATTENTION_CHIPS) {
    out[chip.id] = evaluateQuery(rows, parseQuery(chip.query), ctx).rows.length;
  }
  return out;
}
