/**
 * §8.5.1's gesture, driven. One timer, one phase, and the route derived from both.
 *
 * This lives in a hook rather than in `App.tsx` for the reason `App.tsx` states about itself: it
 * holds no logic of its own, and "which view is on screen while a gesture is mid-flight" is a
 * decision with a wrong answer, not a fact about the window.
 *
 * **The route is derived, never set eagerly.** A click sets the phase; the id the route reads only
 * changes when the view swaps. Setting it on the click would unmount the shelf at 0 ms and leave
 * `crtCollapse`, `shelfRecede` and the beam animating an element nobody can see — which is a hard
 * cut wearing the gesture's timings.
 */
import { useCallback, useEffect, useState } from 'react';

import type { ProjectId } from '../../generated/protocol';
import {
  IDLE,
  gestureRuns,
  nextPhase,
  phaseDurationMs,
  routeProjectFor,
  type TransitionPhase,
} from '../motion/transition';
import type { ResolvedTier } from '../motion/tier';

export interface ProjectTransition {
  /** Where the gesture is. `landing` carries the tile to unfold and the header to flare. */
  readonly phase: TransitionPhase;
  /** The project the route must render, `null` for the shelf. Derived from the phase. */
  readonly projectId: ProjectId | null;
  readonly openProject: (id: ProjectId) => void;
  readonly closeProject: () => void;
}

export function useProjectTransition(tier: ResolvedTier): ProjectTransition {
  const [open, setOpen] = useState<ProjectId | null>(null);
  const [phase, setPhase] = useState<TransitionPhase>(IDLE);

  const openProject = useCallback(
    (id: ProjectId) => {
      // §11.6: below `full` there is no gesture, so there is nothing to wait for. Not a shorter
      // animation — none, and the swap is immediate.
      //
      // And §8.5.1's gesture is shelf→project. Following a link from one page to another is not
      // that gesture and must not borrow it: `opening` puts the shelf back on the route, so a
      // page-to-page move would flash the shelf for 620 ms on its way to a page.
      if (!gestureRuns(tier) || open !== null) {
        setPhase(IDLE);
        setOpen(id);
        return;
      }
      setPhase({ kind: 'opening', id });
    },
    [open, tier],
  );

  const closeProject = useCallback(() => {
    if (open === null) return;
    if (!gestureRuns(tier)) {
      setPhase(IDLE);
      setOpen(null);
      return;
    }
    setPhase({ kind: 'closing', id: open });
  }, [open, tier]);

  useEffect(() => {
    const ms = phaseDurationMs(phase);
    if (ms === null) return undefined;
    const timer = setTimeout(() => {
      // The swap instants. `opening` ends by showing the page it collapsed into; `closing` ends by
      // showing the shelf, which then holds the landing state for its own duration.
      if (phase.kind === 'opening') setOpen(phase.id);
      if (phase.kind === 'closing') setOpen(null);
      setPhase(nextPhase(phase));
    }, ms);
    return () => {
      clearTimeout(timer);
    };
  }, [phase]);

  return { phase, projectId: routeProjectFor(phase, open), openProject, closeProject };
}
