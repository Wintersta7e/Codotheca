import type {
  CheckState,
  CompletionCheck,
  CompletionCheckRow,
  UnknownReason,
} from '../../../generated/protocol';

/**
 * [p3] §31.7 — **where the ten ticks actually live.**
 *
 * `concept.md` says ten discrete ticks on the card. **The design never built them**: §7.7's two
 * band tables allocate every pixel of the plate and contain no tick band, and the prototype
 * computes `ticks` and `showTicks` and renders neither anywhere. The *content* survives intact —
 * ten checks, discrete, lit and unlit, with unknown drawn as neither — and only the **placement**
 * is superseded.
 */

/** The ten rendered labels. One owner, so a label cannot drift from the key it names. */
export const CHECK_LABELS: Readonly<Record<CompletionCheck, string>> = {
  remote: 'REMOTE',
  readme: 'README',
  license: 'LICENSE',
  description: 'DESCRIPTION',
  tests: 'TESTS',
  ci: 'CI',
  ciGreen: 'CI GREEN',
  pushed: 'PUSHED',
  deps: 'DEPENDENCIES',
  release: 'RELEASE',
};

export interface CheckMark {
  readonly glyph: '▣' | '□' | '◌' | '–';
  /**
   * **A check that is `unknown` is drawn hollow-ringed and never as a dark tick.** A dark tick is
   * `fail`, and that is the invariant.
   */
  readonly hollow: boolean;
}

const MARKS: Readonly<Record<CheckState, CheckMark>> = {
  pass: { glyph: '▣', hollow: false },
  fail: { glyph: '□', hollow: false },
  unknown: { glyph: '◌', hollow: true },
  na: { glyph: '–', hollow: false },
};

/** The four marks, transcribed from the design's own specification rather than reinvented. */
export function markFor(row: Pick<CompletionCheckRow, 'state'>): CheckMark {
  return MARKS[row.state];
}

/**
 * §31.7a's note for each reason.
 *
 * **The prototype writes `UNKNOWN · NEEDS GITHUB` for every unknown, which is wrong for six of
 * the ten checks and is not transcribed.** A `license` a budget exceedance left unread has
 * nothing to do with an account, and rendering it as though it did is *claiming currency you do
 * not have* about why a thing is missing.
 *
 * **The split between the last three is load-bearing.** `notObserved` is a fact about the
 * repository and never resolves on its own; `notRunYet` resolves at the next sweep, and
 * `unreachable` when the drive comes back. Collapsing them would make a self-resolving state
 * read as a permanent one.
 */
const NOTES: Readonly<Record<UnknownReason, string>> = {
  needsAccount: 'UNKNOWN · NEEDS AN ACCOUNT',
  notSynced: 'UNKNOWN · NOT SYNCED',
  notRead: 'UNKNOWN · NOT READ',
  notObserved: 'UNKNOWN · NOT OBSERVED',
  notRunYet: 'UNKNOWN · NOT RUN YET',
  unreachable: 'UNKNOWN · UNREACHABLE',
};

/**
 * The note a row renders beneath its label, or `null` for a state that needs none.
 *
 * **The `na` split exists so a user can tell their own decision from the app's** — the render
 * half of the same rule that stores them as two facts. `userNa === true` is the user's ruling;
 * anything else in the `na` state is the archetype's proposal.
 */
export function noteFor(
  row: Pick<CompletionCheckRow, 'state' | 'userNa' | 'unknownReason'>,
): string | null {
  if (row.state === 'na') {
    return row.userNa === true ? 'MARKED NOT APPLICABLE' : 'NOT APPLICABLE';
  }
  if (row.state !== 'unknown') return null;
  // A stored `unknown` always carries a reason — the DDL pairs them — so a missing one is a row
  // this build cannot explain, and saying so is more honest than inventing a cause.
  return row.unknownReason === null ? 'UNKNOWN' : NOTES[row.unknownReason];
}
