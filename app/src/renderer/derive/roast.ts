/**
 * §5.6 — the roast producer. Five stored facts, one line, first match wins.
 *
 * A pure function of the `projects.get` payload: no column, no command, no event, no cached
 * string, and no work when the switch is off. Because it is recomputed on every open it cannot
 * become the stalest thing on the page.
 *
 * Scope is hard: the block renders only inside an opened project card. Never on the grid, never
 * in Peek, never in the list, never in quick switch, never in the scan summary, never in a
 * notification. Nothing else may import this module.
 *
 * And nothing it produces may mention an absence or praise a volume: there is no line for a
 * repository with nothing outstanding, because silence is the honest render and a completion
 * claim is not.
 */

import type { InterruptedOp, Presence } from '../../generated/protocol';
import { asOfClause, formatAge, ROAST_DIRTY_MAX_AGE_SECS } from './observation';

/**
 * The two operations §5.6 gives phase 1 a sentence for.
 *
 * Narrowed from the generated `InterruptedOp` rather than restated, so it can only ever hold a
 * value the wire carries: the schema declares six, plan 05's reader produces two, and a
 * hand-written pair here could drift into naming a third that no sentence exists for.
 */
export type RoastableOp = Extract<InterruptedOp, 'merge' | 'rebase'>;

export interface ShownLocation {
  locationId: string;
  presence: Presence;
  interruptedOp: RoastableOp | null;
  /** null = never observed. */
  isDirty: boolean | null;
  worktreeObservedAt: number | null;
  ahead: number | null;
  /** null = no fetch has ever been recorded here. Never rendered as 0 and never as an age. */
  fetchHeadAt: number | null;
  stashCount: number | null;
  headOid: string | null;
  lastCommitAt: number | null;
}

export interface PrimaryRef {
  locationId: string;
  headOid: string | null;
}

export interface RoastInput {
  roastsEnabled: boolean;
  isReference: boolean;
  isArchived: boolean;
  /** §1.2's never-succeeded state: error_kind set with no observation ever recorded. */
  neverSucceeded: boolean;
  /** The location the page is showing — the one PLAY launches. Never §5.1's aggregate. */
  shown: ShownLocation;
  primary: PrimaryRef | null;
  now: number;
}

const STALE_COMMIT_SECS = 30 * 86_400;

export function roastLine(input: RoastInput): string | null {
  const { shown, now } = input;
  if (
    !input.roastsEnabled ||
    input.isReference ||
    input.isArchived ||
    input.neverSucceeded ||
    shown.presence !== 'present'
  ) {
    // §4.6: absent is not abandoned. A location that could not be read says nothing.
    return null;
  }

  // 1. An open operation outranks everything: it is the one state in which the other four
  //    numbers were measured mid-operation.
  if (shown.interruptedOp !== null) {
    return `A ${shown.interruptedOp} is open here and unfinished.`;
  }

  // 2. Uncommitted work exists in no object database at all. It is the only clause resting on
  //    worktree state, so it is the only one carrying a time and the only one dropped for age:
  //    a sentence has no room for a hedge wide enough to cover a day-old reading.
  if (shown.isDirty === true && shown.worktreeObservedAt !== null) {
    const age = now - shown.worktreeObservedAt;
    if (age < ROAST_DIRTY_MAX_AGE_SECS) {
      return `Uncommitted work here ${asOfClause(shown.worktreeObservedAt, now)}.`;
    }
  }

  // 3. Local-only commits on a branch the user meant to publish. The fetch age is stored
  //    (`location.fetch_head_at`); with none recorded, `ahead` is a count against a ref of
  //    unknown age, which a sentence may not state as a number.
  if (
    shown.ahead !== null &&
    shown.ahead > 0 &&
    shown.fetchHeadAt !== null &&
    shown.lastCommitAt !== null &&
    now - shown.lastCommitAt > STALE_COMMIT_SECS
  ) {
    const newest = formatAge(now - shown.lastCommitAt);
    const fetched = formatAge(now - shown.fetchHeadAt);
    return `${shown.ahead} commits ahead of its upstream, the newest ${newest} old. Last fetch ${fetched} ago.`;
  }

  // 4. A stash is parked by construction, so it ranks below a branch that is not.
  if (shown.stashCount !== null && shown.stashCount > 0) {
    return `${shown.stashCount} stashes here. Nothing pushes a stash.`;
  }

  // 5. Last on principle: two working copies are a deliberate arrangement, not a fault.
  //    Compared by identity, never by direction; a NULL on either side is "not computed".
  if (
    input.primary !== null &&
    input.primary.locationId !== shown.locationId &&
    shown.headOid !== null &&
    input.primary.headOid !== null &&
    shown.headOid !== input.primary.headOid
  ) {
    return 'This is not the copy the app treats as primary. The two are at different commits.';
  }

  return null;
}
