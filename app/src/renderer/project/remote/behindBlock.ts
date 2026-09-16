/**
 * §25.1's `BEHIND`, and it is deliberately the **local** figure.
 *
 * One owner per value: `location.behind` is already stored, and a compare call per project per
 * sync is a rate-budget cost (§21) for a number the index holds. It is also what gives the tab
 * content with **no account connected**, which is what keeps the tab from being the surface that
 * exists only to advertise a feature.
 *
 * `fetchClause` is **imported** from §8.5.2's own module rather than re-written here (R12,
 * AC-P2-25-5 asserts the import). The two surfaces differ by design and only in one place:
 * `locationFacts` omits `BEHIND` at a measured zero because §8.5.2's list has no room for a null
 * statement, while §25.1's block renders `0` with the sub-line `no known divergence`. Same stored
 * value, same fetch clause, a different surface's rule — one value with two renderings, not two
 * values (R15: compare shapes, not strings).
 */
import type { LocationDetail } from '../../../generated/protocol';
import { fetchClause } from '../locations/locationCopy';

export interface BehindFact {
  /** `null` when nothing has compared this copy — the block renders `—` rather than `0`. */
  readonly behind: number | null;
  readonly subLine: string;
}

/**
 * The block's inputs for the **shown** location, or `null` when the page is showing none.
 *
 * `no known divergence` is the meaning of a measured zero, and it is a claim about the last
 * fetch rather than about now — which is why the fetch clause travels with it in every case.
 */
export function behindFact(location: LocationDetail | null, now: number): BehindFact | null {
  if (location === null) return null;
  const clause = fetchClause(location.fetchHeadAt, now);
  const meaning = location.behind === 0 ? 'no known divergence · ' : '';
  return { behind: location.behind, subLine: `${meaning}${clause}` };
}
