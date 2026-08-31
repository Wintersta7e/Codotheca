/**
 * §7.4's explicit reroll — the one user-initiated re-seed in the product. It lives in this rail
 * and nowhere else: never on the grid, never on a selection, and there is no reroll of a section,
 * a collection or the shelf.
 *
 * **One control**, a stepper in the sense the density control is one: the forward `↻` always; the
 * leading `↺` back step and the position readout **only once the offset is above 0** — at 0 there
 * is nowhere back to go, and a greyed step is exactly the dead control §11.3a forbids, so it is
 * absent rather than disabled, as this page's two missing tabs are.
 *
 * **Absolute, never an increment.** The renderer sends the target offset it computed, so a
 * retried, replayed or double-delivered message writes the same integer and lands on the same
 * card. A step more than one from the stored value comes back rejected carrying that stored
 * value, and the rail adopts it — a stale rail resyncs instead of teleporting the walk.
 */
import { useState, type ReactElement } from 'react';

import type { ProjectId } from '../../../generated/protocol';
import { useProjectPageDeps } from '../deps';

export function nextOffset(current: number, direction: 1 | -1): number {
  return Math.max(0, current + direction);
}

/** §7.3a's seed string: `basename` at 0, `basename#<n>` above it. */
export function readout(offset: number, seedBasename: string): string | null {
  return offset === 0 ? null : `${seedBasename}#${String(offset)}`;
}

export interface RerollStepperProps {
  projectId: ProjectId;
  seedBasename: string;
  offset: number;
  onOffset: (offset: number) => void;
}

export function RerollStepper({
  projectId,
  seedBasename,
  offset,
  onOffset,
}: RerollStepperProps): ReactElement {
  const deps = useProjectPageDeps();
  const [busy, setBusy] = useState(false);

  const step = (direction: 1 | -1): void => {
    if (busy) return;
    const target = nextOffset(offset, direction);
    if (target === offset) return;
    setBusy(true);
    deps
      .request('art.rerender', { projectId, offset: target })
      .then((result) => {
        // Accepted or rejected, the reply carries where the walk now stands.
        onOffset(result.offset);
      })
      .catch(() => {
        // The stored value is unchanged; the rail keeps the offset it already had.
      })
      .finally(() => {
        setBusy(false);
      });
  };

  const position = readout(offset, seedBasename);

  return (
    <div className="cp-reroll">
      {offset > 0 ? (
        <button
          type="button"
          className="cp-reroll-step"
          data-testid="cp-reroll-back"
          aria-label="Step back one card art"
          onClick={() => {
            step(-1);
          }}
        >
          <span className="cp-reroll-glyph" aria-hidden="true">
            ↺
          </span>
        </button>
      ) : null}
      {position === null ? null : (
        <span className="cp-reroll-readout" data-testid="cp-reroll-readout">
          {position}
        </span>
      )}
      <button
        type="button"
        className="cp-reroll-step cp-reroll-fwd"
        data-testid="cp-reroll-fwd"
        aria-label="Reroll this card art"
        onClick={() => {
          step(1);
        }}
      >
        REROLL
        <span className="cp-reroll-glyph" aria-hidden="true">
          ↻
        </span>
      </button>
    </div>
  );
}
