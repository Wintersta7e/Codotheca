/**
 * §5.6's producer, rendered in §8.5.1's chrome. This module is the producer's only importer and
 * the page shell is this module's only importer — that pair is the scope invariant, and it is
 * enforced by a test rather than by convention. The block appears only inside an opened project:
 * never on the grid, never in Peek, never in the list, never in the palette, never in triage.
 *
 * Nothing here is stored and nothing new crosses the wire: the line is a pure function of the
 * detail payload, recomputed on every open, so it cannot become the stalest thing on the page.
 */
import type { ReactElement } from 'react';

import type { LocationDetail, ProjectDetail } from '../../generated/protocol';
import { roastLine, type PrimaryRef, type RoastInput, type ShownLocation } from '../derive/roast';
// §1.2's never-succeeded state is declared once, beside the badge that is also produced from it.
// A second copy here would let one project read as *never indexed* to the badge and *indexed* to
// this note in the same frame.
import { neverSucceeded } from '../errors/errorKind';
import { useProjectPageDeps } from './deps';

function toShown(location: LocationDetail, lastCommitAt: number | null): ShownLocation {
  return {
    locationId: String(location.location.id),
    presence: location.presence,
    // §5.6 has a sentence for two operations. The wire carries six, and a third would be phrased
    // by nothing at all.
    interruptedOp:
      location.interruptedOp === 'merge' || location.interruptedOp === 'rebase'
        ? location.interruptedOp
        : null,
    isDirty: location.isDirty,
    worktreeObservedAt: location.worktreeObservedAt,
    ahead: location.ahead,
    fetchHeadAt: location.fetchHeadAt,
    stashCount: location.stashCount,
    headOid: location.headOid,
    lastCommitAt,
    // [p2] §24.6a: carried through so the roast can be suppressed. Re-deriving *removed* from
    // `presence` here would be a second owner for a fact `presence` cannot express.
    removedAt: location.removedAt,
  };
}

/**
 * §5.6 phrases from the location the page is showing, never §5.1's aggregate: with three copies
 * of one project the aggregate's `is_dirty` belongs to a folder the sentence is not about.
 */
export function toRoastInput(args: {
  detail: ProjectDetail;
  shown: LocationDetail | null;
  primary: LocationDetail | null;
  roastsEnabled: boolean;
  now: number;
}): RoastInput | null {
  const { detail, shown, primary, roastsEnabled, now } = args;
  if (shown === null) return null;
  const primaryRef: PrimaryRef | null =
    primary === null ? null : { locationId: String(primary.location.id), headOid: primary.headOid };
  return {
    roastsEnabled,
    isReference: detail.row.isReference,
    isArchived: detail.row.isArchived,
    neverSucceeded: neverSucceeded(detail.row),
    shown: toShown(shown, detail.row.lastCommitAt),
    primary: primaryRef,
    now,
  };
}

export interface RoastNoteProps {
  detail: ProjectDetail;
  shown: LocationDetail | null;
  primary: LocationDetail | null;
  roastsEnabled: boolean;
}

export function RoastNote({
  detail,
  shown,
  primary,
  roastsEnabled,
}: RoastNoteProps): ReactElement | null {
  const deps = useProjectPageDeps();
  const input = toRoastInput({ detail, shown, primary, roastsEnabled, now: deps.now() });
  if (input === null) return null;
  const line = roastLine(input);
  // Nothing matched: no block at all, not an empty block and not a placeholder. Silence is the
  // honest render, and a "nothing outstanding" line would be a completion claim.
  if (line === null) return null;

  return (
    <div className="cp-roast" data-testid="cp-roast">
      <span className="cp-roast-label" data-testid="cp-roast-label">
        NOTE
      </span>
      <span className="cp-roast-line" data-testid="cp-roast-line">
        {line}
      </span>
    </div>
  );
}
