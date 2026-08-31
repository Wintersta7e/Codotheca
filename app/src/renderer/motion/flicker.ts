import { useEffect, useRef, useState } from 'react';
import type { ProjectId, ProjectRow } from '../../generated/protocol';
import { type ResolvedTier, allowsScheduledFrames } from './tier';
import { useWindowActive } from './useWindowActive';

/**
 * §11.6's scheduled flicker — **the one permitted scheduled frame in phase 1**, and the only
 * exception criterion 21 admits. §11.6 owns every parameter here and no other section states a
 * depth, a duration, a period or an eligibility predicate; the design's fixed 50% / 130 ms /
 * 9,000 ms are cut, and a test asserting any of the three tests a superseded specification.
 *
 * The dip arrives as a **custom property**, never as an inline `opacity`: `motion.css` clamps
 * `[data-effects-tier='reduced'] .cdt-card-halo { opacity: 1 }`, and an inline value outranks
 * every selector, so the dip would keep running at the two tiers that forbid it.
 */
export const FLICKER_MEAN_MS = 9000;
export const FLICKER_DEPTH_RANGE: readonly [0.15, 0.25] = [0.15, 0.25];
export const FLICKER_HOLD_MS_RANGE: readonly [80, 140] = [80, 140];
export const FLICKER_DIP_COUNT_RANGE: readonly [1, 2] = [1, 2];
export const FLICKER_OPACITY_PROPERTY = '--cdt-halo-opacity';

/** Gap between dips inside one event. Not a §11.6 parameter: it is the event's own shape. */
const INTER_DIP_MS = 70;

export type FlickerRow = Pick<ProjectRow, 'conditionSignal' | 'isReference' | 'presence'>;

export function flickerEligible(row: FlickerRow): boolean {
  if (row.isReference) return false;
  if (row.presence !== 'present') return false;
  return row.conditionSignal === 'neglected' || row.conditionSignal === 'abandoned';
}

/** Deterministic, seeded per session, so a run is reproducible and a test can pin it. */
export function flickerRandom(seed: number): () => number {
  let state = seed >>> 0;
  return (): number => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const between = (rand: () => number, lo: number, hi: number): number => lo + rand() * (hi - lo);

export interface FlickerStep {
  readonly atMs: number;
  readonly opacity: number;
}

export interface FlickerEvent {
  /** Which candidate dips. **One card**, and the caller has already intersected the viewport. */
  readonly projectIndex: number;
  readonly steps: readonly FlickerStep[];
  readonly endMs: number;
}

export function nextFlickerEvent(
  rand: () => number,
  afterMs: number,
  candidateCount: number,
): FlickerEvent {
  // The exponential gap of a Poisson process at FLICKER_MEAN_MS.
  const gap = -FLICKER_MEAN_MS * Math.log(1 - rand());
  const dips =
    FLICKER_DIP_COUNT_RANGE[0] +
    Math.floor(rand() * (FLICKER_DIP_COUNT_RANGE[1] - FLICKER_DIP_COUNT_RANGE[0] + 1));
  const steps: FlickerStep[] = [];
  let at = afterMs + gap;
  for (let i = 0; i < dips; i += 1) {
    const depth = between(rand, FLICKER_DEPTH_RANGE[0], FLICKER_DEPTH_RANGE[1]);
    const hold = between(rand, FLICKER_HOLD_MS_RANGE[0], FLICKER_HOLD_MS_RANGE[1]);
    steps.push({ atMs: at, opacity: 1 - depth });
    at += hold;
    steps.push({ atMs: at, opacity: 1 });
    at += INTER_DIP_MS;
  }
  return {
    projectIndex: candidateCount === 0 ? 0 : Math.floor(rand() * candidateCount),
    steps,
    endMs: at,
  };
}

export interface FlickerDeps {
  readonly tier: ResolvedTier;
  /** Seeded per session (§11.6), so a run is reproducible. */
  readonly seed: number;
  readonly monotonicMs: () => number;
}

export interface FlickerState {
  readonly projectId: ProjectId | null;
  readonly opacity: number;
}

const STEADY: FlickerState = { projectId: null, opacity: 1 };

/**
 * At most one card dips, and only one that is **in the viewport** — mounted is not enough, since
 * a virtualizer keeps off-screen rows mounted and a collapsed section has nothing to dip. The
 * caller supplies the intersection.
 *
 * Suspended while unfocused or hidden **with no queued backlog**: the schedule resumes from now
 * rather than replaying what it missed, and the seeded stream survives the suspension rather
 * than restarting, which is what "seeded per session" means. Between edges nothing is scheduled
 * — the 90 ms transition in `card.css` interpolates the dip, so there is no animation-frame loop
 * anywhere in phase 1.
 */
export function useFlicker(candidates: readonly ProjectId[], deps: FlickerDeps): FlickerState {
  const active = useWindowActive();
  const [state, setState] = useState<FlickerState>(STEADY);
  const running = active && allowsScheduledFrames(deps.tier) && candidates.length > 0;

  /**
   * The viewport list and the clock are read at **event time** and are deliberately not
   * dependencies. Plan 13 recomputes `visibleProjectIds` on every scroll frame; a schedule keyed
   * on that array's identity — or even on its length — reseeds its ~9 s gap on every one of
   * those renders and emits no dip at all. Only `running` restarts it, and that is a boolean
   * which flips when the tier changes, the window's activity changes, or the candidate set
   * empties.
   */
  const candidatesRef = useRef(candidates);
  const clockRef = useRef(deps.monotonicMs);
  useEffect(() => {
    candidatesRef.current = candidates;
    clockRef.current = deps.monotonicMs;
  });

  // One stream per session, not one per suspension.
  const randomRef = useRef<() => number>(flickerRandom(deps.seed));
  const seedRef = useRef(deps.seed);
  if (seedRef.current !== deps.seed) {
    seedRef.current = deps.seed;
    randomRef.current = flickerRandom(deps.seed);
  }

  useEffect(() => {
    if (!running) {
      setState(STEADY);
      return undefined;
    }
    const rand = randomRef.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;

    const schedule = (fromMs: number): void => {
      const now = clockRef.current;
      const pool = candidatesRef.current;
      const event = nextFlickerEvent(rand, fromMs, pool.length);
      const target = pool[event.projectIndex] ?? null;
      let step = 0;

      const advance = (): void => {
        if (stopped) return;
        const current = event.steps[step];
        if (current === undefined) {
          // The event is spent. The next gap starts from now, never from the time the event
          // was scheduled for: a suspended window resumes, it does not catch up.
          schedule(now());
          return;
        }
        step += 1;
        setState(target === null ? STEADY : { projectId: target, opacity: current.opacity });
        // The wake is the NEXT step's time, not this one's. Re-arming on the step just played
        // leaves a zero delay, and the dip and its return land in the same tick — every
        // parameter correct and nothing visible on screen.
        const next = event.steps[step];
        timer = setTimeout(advance, next === undefined ? 0 : Math.max(0, next.atMs - now()));
      };

      const startAt = event.steps[0]?.atMs ?? event.endMs;
      timer = setTimeout(advance, Math.max(0, startAt - fromMs));
    };

    schedule(clockRef.current());
    return (): void => {
      stopped = true;
      if (timer !== undefined) clearTimeout(timer);
      setState(STEADY);
    };
  }, [running]);

  return state;
}
