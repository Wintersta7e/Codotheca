import { describe, expect, it } from 'vitest';
import type { DebtItem, DebtScoring, DebtItemState, DecayLayer } from '../../generated/protocol';
import { litAnchorCount, litCounts } from './lit';

function item(
  layer: DecayLayer,
  state: DebtItemState = 'open',
  scoring: DebtScoring = 'scored',
  fingerprint = `${layer}-${state}-${scoring}-${String(Math.random())}`,
): DebtItem {
  return {
    source: 'todo_marker',
    fingerprint,
    state,
    scoring,
    layer,
    pathDisplay: null,
    line: null,
    column: null,
    salientText: null,
    firstSeenAt: 1,
    lastSeenAt: 2,
    basis: 'worktree',
    advisory: null,
  };
}

describe('the lit set', () => {
  it('counts an open item at either scoring, and both on one layer', () => {
    // A7: an advisory with no fix available is visible damage that holds no project off gold.
    // Making it invisible would be the unknown-as-zero rule broken from the other end.
    const counts = litCounts([item('rust', 'open', 'scored'), item('rust', 'open', 'shown_only')]);
    expect(counts.get('rust')).toBe(2);
  });

  it('counts an unverified item on no layer', () => {
    // §28's two states are about OBSERVATION. A layer count is an aggregate, and F10 rules that
    // `unverified` is never a count on a ranked or aggregated surface. §30's item list still
    // renders it — it is the carrier of record.
    const counts = litCounts([item('dust', 'unverified'), item('dust', 'unverified')]);
    expect(counts.get('dust')).toBeUndefined();
  });

  it('counts a closed item on no layer, because a closed item is not in the list', () => {
    // *Closed* is an event, never a state: the row is deleted. An empty list is an empty map.
    expect(litCounts([]).size).toBe(0);
  });

  it('groups by DebtItem.layer and holds no table of its own', () => {
    const counts = litCounts([
      item('dust'),
      item('cobwebs'),
      item('rust'),
      item('rust'),
      item('cracks'),
      item('overgrowth'),
    ]);
    expect([...counts.entries()].sort()).toEqual([
      ['cobwebs', 1],
      ['cracks', 1],
      ['dust', 1],
      ['overgrowth', 1],
      ['rust', 2],
    ]);
  });

  it('saturates at the anchor count, so five and fifty draw the same card', () => {
    // No sprite density, no randomness, no second constant. A card has as many vents as it has.
    expect(litAnchorCount(5, 3)).toBe(3);
    expect(litAnchorCount(50, 3)).toBe(3);
    expect(
      litAnchorCount(litCounts(Array.from({ length: 40 }, () => item('dust'))).get('dust') ?? 0, 2),
    ).toBe(2);
    expect(litAnchorCount(2, 7)).toBe(2);
    expect(litAnchorCount(0, 7)).toBe(0);
  });

  it('lights nothing when a layer has no anchor, and invents none', () => {
    // A1b: an empty anchor list is never repaired by an invented coordinate.
    expect(litAnchorCount(9, 0)).toBe(0);
  });

  /**
   * **`AC-P3-33-2`.** The cobwebs predicate is **§28's** and is pointed at, never restated
   * (R130/F4). §28's producer opens at most one `abandoned_with_debt` item and applies §28.2's
   * conjunct — narrowed to `scored` items by R122 — so counting open items gives cobwebs its
   * 0-or-1 for free and this module has no cobwebs branch at all.
   *
   * Each fixture is the item list §28's producer would have written for that combination. The
   * predicate is never re-derived here: a half-copy is worse than a whole one.
   */
  it('gets cobwebs 0-or-1 from §28s item list, with no predicate of its own', () => {
    const abandonedWithDebt = [item('cobwebs'), item('rust'), item('dust')];
    const abandonedNoDebt = [item('rust')];
    const notAbandonedWithDebt = [item('rust'), item('dust')];
    const notAbandonedNoDebt: DebtItem[] = [];

    expect(litCounts(abandonedWithDebt).get('cobwebs')).toBe(1);
    expect(litCounts(abandonedNoDebt).get('cobwebs')).toBeUndefined();
    expect(litCounts(notAbandonedWithDebt).get('cobwebs')).toBeUndefined();
    expect(litCounts(notAbandonedNoDebt).get('cobwebs')).toBeUndefined();
  });

  /**
   * **R122's live counterexample, which §32 guarantees exists.** A Done project whose only debt
   * is one `shown_only` no-fix advisory lights `rust` and is never cobwebbed — and the
   * discriminator is **`scoring`, not the count**, so a later author cannot satisfy this by
   * reintroducing a count gate.
   *
   * *A verdict about the user's diligence and a depiction of the artifact's condition are two
   * statements, and only the first is Done's.*
   */
  it('lights rust on a Done project carrying one unfixable advisory', () => {
    const doneWithAdvisory = [item('rust', 'open', 'shown_only')];
    const lit = litCounts(doneWithAdvisory);
    expect(lit.get('rust')).toBe(1);
    expect(lit.get('cobwebs')).toBeUndefined();

    // §28's producer opens the cobwebs item once a `scored` item exists. Same count of items on
    // `rust` either side, so the count is not what moved.
    const alsoScored = [item('rust', 'open', 'shown_only'), item('cobwebs')];
    expect(litCounts(alsoScored).get('cobwebs')).toBe(1);
    expect(litCounts(alsoScored).get('rust')).toBe(1);
  });

  it('reads no date, no condition signal and no exclusion', () => {
    // §30 applied the exclusions once, upstream. §33 renders what it is handed — including for
    // an offline project, whose list §30 froze. Rendering a frozen list as clean metal is
    // *unknown as zero* applied to the layer set.
    const frozen = [item('dust'), item('cracks')];
    expect(litCounts(frozen).get('dust')).toBe(1);
    expect(litCounts(frozen).get('cracks')).toBe(1);
  });
});
