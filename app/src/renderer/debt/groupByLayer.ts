/**
 * §28.9's flat debt list, grouped by layer — **R120's renderer half.**
 *
 * The core sends **one flat list**, never five arrays. Five arrays are five places for a layer to
 * be absent-versus-empty, which is the distinction §33.8 already rules on for `WeatherLayer`:
 * *"an absent entry and an empty one would be two spellings of the same fact."* The grouping is a
 * pure function of a field every item already carries.
 */

import type { DebtItem, DecayLayer } from '../../generated/protocol';

/**
 * Group one project's items by `layer`.
 *
 * **A layer with no items is ABSENT from the map, never an empty array.** Seeding all five would
 * reintroduce the two spellings this shape exists to remove: a consumer could not then tell
 * *nothing here* from *nothing computed*, and §33's geometry would draw a layer nobody measured.
 *
 * `unverified` items are **carried through** — the list must be able to say *this item is not
 * being counted right now* — and are counted by nothing here.
 */
export function groupByLayer(items: readonly DebtItem[]): Map<DecayLayer, DebtItem[]> {
  const out = new Map<DecayLayer, DebtItem[]>();
  for (const item of items) {
    const existing = out.get(item.layer);
    if (existing) {
      existing.push(item);
    } else {
      out.set(item.layer, [item]);
    }
  }
  return out;
}

/**
 * The count that enters §30's health reading: `open` **and** `scored`.
 *
 * **The only counting function in this module**, and it is never combined with a check count
 * (A12b) — check counts and item counts are two units, and *never render two units as one* is
 * *never render unknown as zero*'s sibling. `unverified` and `shown_only` items render and are
 * counted by nothing, which is what keeps them off every ranked and aggregated surface.
 */
export function scoredOpenIn(items: readonly DebtItem[]): number {
  let n = 0;
  for (const item of items) {
    if (item.state === 'open' && item.scoring === 'scored') {
      n += 1;
    }
  }
  return n;
}
