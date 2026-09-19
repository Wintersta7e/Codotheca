/**
 * §33.2 — which layers are lit, and by how much.
 *
 * **The mapping is A4's nine-row registry, normative in §28 and carried on `DebtItem.layer`.**
 * This file reads the field and holds no table: a second copy of that registry would be a second
 * place for it to stop being true.
 *
 * **There is no cobwebs branch here, and that is the point.** §28's producer opens at most one
 * `abandoned_with_debt` item and applies §28.2's conjunct — narrowed to `scored` items by R122 —
 * so counting open items gives cobwebs its 0-or-1 for free. §33 states no second copy of that
 * predicate: a half-copy is worse than a whole one (R130/F4).
 *
 * **No exclusion is re-applied and no date is read.** Done, Stable, Archived, Reference and
 * backlog-suppressed projects render as clean metal because §30 applied the exclusions once,
 * upstream, when the list was produced. An `offline` location is not an exception: §30 freezes
 * the list and §33 renders what it is handed — rendering a frozen project as clean metal would
 * be *unknown as zero* applied to the layer set.
 */
import type { DebtItem, DecayLayer } from '../../generated/protocol';

/**
 * Open items per layer. **Both `scoring` values count** (A7): an advisory with no fix available
 * is visible damage that holds no project off the gold-shaped verdict, and making it invisible
 * would be the unknown-as-zero rule broken from the other end.
 *
 * `unverified` items light nothing. §28's two states are about *observation*; a layer count is an
 * aggregate, and `unverified` is never a count on a ranked or aggregated surface (R128/F10). It
 * still renders in §30's item list, which is the carrier of record.
 *
 * A *closed* item is an event, not a state — the row is deleted — so it is absent from the list
 * and counts on nothing without a branch here saying so.
 */
export function litCounts(debt: readonly DebtItem[]): ReadonlyMap<DecayLayer, number> {
  const counts = new Map<DecayLayer, number>();
  for (const entry of debt) {
    if (entry.state !== 'open') continue;
    counts.set(entry.layer, (counts.get(entry.layer) ?? 0) + 1);
  }
  return counts;
}

/**
 * How many anchors a layer with `value` open items lights: the first `min(value, anchors)`, in
 * §33.3's declared order. **The one place saturation is expressed.**
 *
 * No sprite density, no randomness, no second constant. Forty TODOs do not bury the card because
 * a card has as many vents as it has, and the difference between five and fifty is not drawn. A
 * *rate* is rejected: it needs a second stored history and it makes closing one item sometimes
 * *increase* the drawn decay.
 *
 * Zero anchors lights zero. An empty anchor list is never repaired by an invented coordinate
 * (A1b) — no `(0,0)` fallback, no synthetic rect, no "at least one" floor.
 */
export function litAnchorCount(value: number, anchors: number): number {
  return Math.min(Math.max(value, 0), Math.max(anchors, 0));
}
