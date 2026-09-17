/**
 * §25.1's four states of a forge-derived value, in one component.
 *
 * The four states are a **wire enum** and not a rule this file re-derives: `RemoteFactsState`
 * arrives on `RemoteFacts`, and the only thing computed here is staleness, because staleness is
 * a function of *now*.
 *
 * `—` is `UNCOMPUTED_FACT`, §8.4.1's glyph, **imported rather than re-typed**. The string
 * `UNKNOWN` appears nowhere on this surface: the design's `UNKNOWN · NEEDS GITHUB` is phase 3's
 * check treatment, and importing it here would be two glyphs for one meaning.
 */
import type { ReactElement } from 'react';

import type { RemoteFactsState } from '../../../generated/protocol';
import { formatAge, REMOTE_STALE_AFTER_SECS } from '../../derive/observation';
import { UNCOMPUTED_FACT } from '../../shelf/peekText';

export const NOT_YET_FETCHED = 'NOT YET FETCHED';
export const NOT_PERMITTED = 'NOT PERMITTED';

/**
 * The state of **one value** inside a read.
 *
 * A read can complete and still carry no number for a given block — the response simply did not
 * separate that count. That value is *not observed*, not zero, and not a different vocabulary:
 * the block renders `—` under the same word it would use had the read never happened.
 */
export function valueState(state: RemoteFactsState, value: number | null): RemoteFactsState {
  if (state !== 'observed') return state;
  return value === null ? 'not_observed' : 'observed';
}

/**
 * The value slot. **Never `0` for an unobserved value, and never `—` for a measured zero** —
 * AC-P2-25-4 names both directions.
 */
export function blockValue(state: RemoteFactsState, value: number | null): string {
  return valueState(state, value) === 'observed' && value !== null
    ? String(value)
    : UNCOMPUTED_FACT;
}

/**
 * The line under the row of forge blocks (§25.1's Observation row).
 *
 * `null` when nothing has been observed: an age with no observation behind it is the currency
 * claim this project exists to refuse.
 */
export function observationLine(observedAt: number | null, now: number): string | null {
  if (observedAt === null) return null;
  const age = Math.max(0, now - observedAt);
  return age >= REMOTE_STALE_AFTER_SECS
    ? `stale · ${formatAge(age)}`
    : `OBSERVED ${formatAge(age)}`;
}

export interface RemoteBlockProps {
  readonly label: string;
  readonly state: RemoteFactsState;
  readonly value: number | null;
  /**
   * The design's sub-line for a read that **completed**, whether or not it carried this value.
   * `null` renders no sub-line at all — never one built from a value nobody observed, which is
   * the invariant's exact failure mode.
   *
   * It survives a `null` value on purpose. `BEHIND` is the case: `behindFact` says the fetch
   * clause "travels with it in every case", and a branch with no upstream has no number while
   * its fetch is still recorded. Discarding the clause there printed `NOT YET FETCHED` over a
   * fetch that happened.
   */
  readonly subLine?: string | null;
}

/**
 * One forge block. Under `no_account` it **does not render**: §8.4.1 dropped `COMPLETION` from
 * Peek on exactly that argument, and a surface that exists to advertise a feature is an ad.
 */
export function RemoteBlock({
  label,
  state,
  value,
  subLine = null,
}: RemoteBlockProps): ReactElement | null {
  if (state === 'no_account') return null;
  // **The note is decided by the READ, not by the value.** `not_observed` means nothing fetched
  // this fact yet, which is what `NOT YET FETCHED` says. A read that *completed* and carried no
  // number for this block is a different thing: the value slot renders `—` and the note is
  // whatever the caller supplies, which for a forge count is nothing at all.
  //
  // Deciding it from `valueState` instead printed `NOT YET FETCHED` directly beneath
  // `OBSERVED 4m` — a false statement about the product's own behaviour, and the *never claim
  // currency you do not have* invariant failing in the opposite direction. Two of §25.1's three
  // forge blocks hit it on every successful read, because `GET /repos/{owner}/{name}` carries no
  // separated issue or PR counts.
  const note =
    state === 'not_observed'
      ? NOT_YET_FETCHED
      : state === 'not_permitted'
        ? NOT_PERMITTED
        : subLine;

  return (
    <div
      className="cp-remote-block"
      data-testid={`cp-remote-${label.toLowerCase().replace(/ /gu, '-')}`}
    >
      <span className="cp-remote-block-label">{label}</span>
      <span className="cp-remote-block-value">{blockValue(state, value)}</span>
      {note === null || note === undefined ? null : (
        <span className="cp-remote-block-sub">{note}</span>
      )}
    </div>
  );
}
