import { useCallback, useEffect, useRef, useState } from 'react';
import type { CollectionId } from '../../generated/protocol.js';

/**
 * §8.8: the first press swaps the sub-line, the second commits, `Esc` or 6,000 ms disarms —
 * the sheet's own auto-clear beat.
 *
 * What this control removes: a saved query. `collections.remove` deletes the `collection` row
 * and its `collection_member` rows and touches nothing else — no `project`, no `location`, no
 * `session`, no `xp_events`, no byte on disk. §17: phase 1 has no destructive operation at all.
 * The second press exists because §1.12 lists collections among the content no rescan can
 * re-derive, not because anything on disk is at risk.
 */
export const ARM_SUB_LINE = 'PRESS AGAIN TO DELETE · THE PROJECTS STAY';

/** §8.8's disarm window. */
export const DISARM_MS = 6_000;

/**
 * §8.8: an armed chip pins its ground and drops the hover fill, because `--warn` measures
 * 6.24:1 on `--surface-2` and 5.73:1 on `--surface-4` — a state whose contrast depends on where
 * the pointer is has already failed §8.7 clause 1.
 */
export const ARMED_GROUND = 'var(--surface-2)';

export type ArmedState =
  { readonly armed: null } | { readonly armed: CollectionId; readonly armedAtMs: number };

export const DISARMED: ArmedState = { armed: null };

export type ArmedAction =
  | { readonly type: 'press'; readonly id: CollectionId; readonly nowMs: number }
  | { readonly type: 'disarm' }
  | { readonly type: 'sweep'; readonly nowMs: number };

export interface ArmedStep {
  readonly state: ArmedState;
  /** The collection to remove, or `null`. Never more than one per step. */
  readonly commit: CollectionId | null;
}

export function isArmed(state: ArmedState, id: CollectionId): boolean {
  return state.armed === id;
}

function withinWindow(state: ArmedState, nowMs: number): boolean {
  return state.armed !== null && nowMs - state.armedAtMs <= DISARM_MS;
}

/**
 * The clock decides, not the timer. A background renderer throttles `setTimeout`, so a stale arm
 * can outlive its own disarm; a second press outside the window re-arms and never deletes.
 */
export function armedDeleteStep(state: ArmedState, action: ArmedAction): ArmedStep {
  switch (action.type) {
    case 'press':
      if (state.armed === action.id && withinWindow(state, action.nowMs)) {
        return { state: DISARMED, commit: action.id };
      }
      return { state: { armed: action.id, armedAtMs: action.nowMs }, commit: null };
    case 'disarm':
      return { state: DISARMED, commit: null };
    case 'sweep':
      return withinWindow(state, action.nowMs)
        ? { state, commit: null }
        : { state: DISARMED, commit: null };
  }
}

export interface ArmedDeleteDeps {
  /** R3: monotonic milliseconds through deps, never `Date.now()` in renderer code. */
  readonly nowMs: () => number;
  readonly setTimer: (cb: () => void, ms: number) => number;
  readonly clearTimer: (handle: number) => void;
  readonly onCommit: (id: CollectionId) => void;
}

export interface ArmedDeleteHandle {
  readonly armedId: CollectionId | null;
  readonly press: (id: CollectionId) => void;
  readonly disarm: () => void;
}

export function useArmedDelete(deps: ArmedDeleteDeps): ArmedDeleteHandle {
  const [state, setState] = useState<ArmedState>(DISARMED);
  const timer = useRef<number | null>(null);
  const depsRef = useRef(deps);
  depsRef.current = deps;

  const cancelTimer = useCallback((): void => {
    if (timer.current !== null) {
      depsRef.current.clearTimer(timer.current);
      timer.current = null;
    }
  }, []);

  useEffect(() => cancelTimer, [cancelTimer]);

  const press = useCallback(
    (id: CollectionId): void => {
      const step = armedDeleteStep(state, {
        type: 'press',
        id,
        nowMs: depsRef.current.nowMs(),
      });
      cancelTimer();
      setState(step.state);
      if (step.commit !== null) {
        depsRef.current.onCommit(step.commit);
        return;
      }
      // The courtesy sweep. It re-checks the clock rather than assuming its own punctuality.
      timer.current = depsRef.current.setTimer(() => {
        timer.current = null;
        setState(
          (current) =>
            armedDeleteStep(current, { type: 'sweep', nowMs: depsRef.current.nowMs() }).state,
        );
      }, DISARM_MS);
    },
    [cancelTimer, state],
  );

  const disarm = useCallback((): void => {
    cancelTimer();
    setState(DISARMED);
  }, [cancelTimer]);

  return { armedId: state.armed, press, disarm };
}
