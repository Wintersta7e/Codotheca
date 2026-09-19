import type { SortKey } from '../../generated/protocol.js';
import type { IgnoredTerm, QueryAst, QueryTerm } from '../../shared/query/ast.js';
import { parseQuery } from '../../shared/query/parse.js';
import type { QueryContext } from './evaluate.js';
import { evaluateQuery } from './evaluate.js';
import type { SectionAggregate } from './eras.js';
import { aggregateSection, eraSectionIdFor, eraSectionLabel, eraSectionOrder } from './eras.js';
import type { ShelfRow } from './row.js';

export interface ShelfSection {
  readonly id: string;
  readonly order: number;
  readonly year: number | null;
  readonly cutAgainstYear: number;
  readonly label: string;
  readonly agg: SectionAggregate;
  readonly rows: readonly ShelfRow[];
}

export interface ShelfPage {
  readonly sections: readonly ShelfSection[];
  /** §8.1: Reference is not an era section. Uncapped — forty is exactly the wrong number to stop at. */
  readonly reference: readonly ShelfRow[];
  readonly ignored: readonly IgnoredTerm[];
  readonly matched: number;
  /** The collapse threshold counts rendered rows, so Reference is excluded from this. */
  readonly renderedTotal: number;
  readonly orderKey: string;
  readonly generation: number;
  readonly ast: QueryAst;
}

/**
 * [p3] §35.3's membership rule and §35.2's ordering scalar, as one total function — the mirror of
 * `rank_of` in `core/src/projects/list.rs`, which `protocol/shelf/order-corpus.json` holds both
 * halves to.
 *
 * A number is *this row carries a reading, and its count is that number*; `null` is *tail*. The
 * case §30.1 forbids the writer to produce — a `live` reading with a null `scoredOpen` — resolves
 * to the tail rather than to a zero, because **a zero is a ranked value and never a tail value**.
 *
 * It re-applies none of §35.4's exclusions: §30's pipeline decides what the reading is, and this
 * reads it. `isReference`, `isArchived` and `lifecycle` are never consulted.
 */
export function rankOf(row: ShelfRow): number | null {
  switch (row.healthSummary.state) {
    case 'live':
    case 'frozen':
      return row.healthSummary.scoredOpen;
    case 'absent':
    case 'suppressed':
      return null;
  }
}

/** §8.0a's default order, shared by `last_touched` and `needs_attention` so one value has one
 *  owner. `name` and `size` keep their own tiebreaks, which are different values. */
function byDefault(a: ShelfRow, b: ShelfRow): number {
  return b.lastTouchedAt - a.lastTouchedAt || a.id - b.id;
}

export function compareRows(sort: SortKey): (a: ShelfRow, b: ShelfRow) => number {
  if (sort === 'name') {
    return (a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || a.id - b.id;
  }
  if (sort === 'size') {
    // A row with no inventory sorts last. It is not a small repository; it is an unmeasured one.
    return (a, b) => {
      const av = a.sizeTrackedBytes;
      const bv = b.sizeTrackedBytes;
      if (av === null && bv === null) return a.id - b.id;
      if (av === null) return 1;
      if (bv === null) return -1;
      return bv - av || a.id - b.id;
    };
  }
  // [p3] §35.3, in `size`'s shape. **This branch has to be explicit**: the function ends in an
  // unconditional return, so a fourth key with no branch of its own silently sorts as
  // `last_touched` while `tsc` stays green — which is why `AC-P3-35-2` was written before it.
  if (sort === 'needs_attention') {
    return (a, b) => {
      const av = rankOf(a);
      const bv = rankOf(b);
      if (av === null && bv === null) return byDefault(a, b);
      if (av === null) return 1;
      if (bv === null) return -1;
      return bv - av || byDefault(a, b);
    };
  }
  return byDefault;
}

/** FNV-1a over the ordered ids. A cursor, not a checksum: equality is all it has to support. */
export function orderKeyOf(ids: readonly number[]): string {
  let hash = 0x811c_9dc5;
  for (const id of ids) {
    for (let shift = 0; shift < 32; shift += 8) {
      hash ^= (id >>> shift) & 0xff;
      hash = Math.imul(hash, 0x0100_0193) >>> 0;
    }
  }
  return hash.toString(16).padStart(8, '0');
}

/**
 * §8.0b's base predicate keeps reference rows out of a bare query, and §8.1 still renders them
 * below the grid, uncapped. Appending the flag the predicate itself looks for opts exactly that
 * one exclusion back in, so the tail runs the *same* predicate as the grid rather than a second
 * one written beside it — which is how the prototype's chips came to disagree with its headline.
 */
const REFERENCE_TERM: QueryTerm = { kind: 'flag', negated: false, flag: 'reference' };

export function buildShelfPage(input: {
  readonly rows: readonly ShelfRow[];
  readonly query: string;
  readonly sort: SortKey;
  readonly now: number;
  readonly generation: number;
  readonly ctx: QueryContext;
}): ShelfPage {
  const ast = parseQuery(input.query);
  const order = compareRows(input.sort);
  const evaluated = evaluateQuery(input.rows, ast, input.ctx);
  const ordered = [...evaluated.rows].sort(order);

  const referenceAst: QueryAst = { ...ast, terms: [...ast.terms, REFERENCE_TERM] };
  const reference = evaluateQuery(input.rows, referenceAst, input.ctx)
    .rows.filter((row) => row.isReference)
    .sort(order);

  const cutAgainstYear = new Date(input.now * 1000).getFullYear();
  const buckets = new Map<string, ShelfRow[]>();

  for (const row of ordered) {
    // Reference never enters an era section, even when the query asked for it: it has one home
    // below the grid, and rendering it in both would count one project twice.
    if (row.isReference) continue;
    const id = eraSectionIdFor(row, input.now);
    const bucket = buckets.get(id);
    if (bucket === undefined) buckets.set(id, [row]);
    else bucket.push(row);
  }

  const sections: ShelfSection[] = [...buckets.entries()]
    .map(([id, rows]) => {
      const yearText = id.slice('era:'.length);
      const year = /^\d{4}$/.test(yearText) ? Number(yearText) : null;
      return {
        id,
        order: eraSectionOrder(id, cutAgainstYear),
        year,
        cutAgainstYear,
        label: eraSectionLabel(id, cutAgainstYear),
        agg: aggregateSection(rows),
        rows,
      };
    })
    .sort((a, b) => a.order - b.order);

  const renderedTotal = sections.reduce((sum, section) => sum + section.agg.count, 0);

  return {
    sections,
    reference,
    ignored: evaluated.ignored,
    matched: ordered.filter((row) => !row.isReference).length,
    renderedTotal,
    // The window addresses the flat section order (§8.2), so that is what the cursor covers.
    orderKey: orderKeyOf(sections.flatMap((section) => section.rows).map((row) => row.id)),
    generation: input.generation,
    ast,
  };
}
