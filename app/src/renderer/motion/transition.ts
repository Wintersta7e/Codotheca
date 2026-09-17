/**
 * §8.5.1's shelf⇄project gesture, as numbers and a phase machine.
 *
 * **The gesture has two owners and one clock.** The shelf runs the tile collapse, the recede and
 * the beam; the project page runs the power-on, the rail slide and the right-column cascade
 * (`project/motion.ts`). Neither can see the other's element, so the only thing that joins them is
 * *when the view swaps* — 620 ms in, 300 ms out. Those two instants live in `project/motion.ts`
 * beside the page's own beats, and are imported here rather than restated: a duration written in
 * two places is one that drifts.
 *
 * Every curve and every keyframe below is the design handoff's prototype verbatim
 * (`design_handoff_codotheca/Codotheca v7 Shelf.dc.html:32-43`), which beats its prose files.
 *
 * §11.6 decides whether any of it runs. At `reduced` and `off` there is no gesture at all and the
 * swap is immediate — not a faster gesture, none: `shelfRecede` blurs and `crtCollapse` flashes
 * brightness to 4.5, and a clamped version of either is still a travelling highlight.
 */
import { PAGE_ENTRY, PAGE_EXIT } from '../project/motion';
import type { ProjectId } from '../../generated/protocol';
import type { ResolvedTier } from './tier';

/** Leaving the shelf. Durations and delays as §8.5.1 states them. */
export const SHELF_EXIT = {
  tileCollapseMs: 420,
  shelfRecedeMs: 500,
  shelfRecedeDelayMs: 100,
  beamStretchMs: 560,
  beamStretchDelayMs: 160,
  beamFlareMs: 580,
  beamFlareDelayMs: 180,
} as const;

/** Coming back to it. `stripFlare` outlasts the landing state and is cleared with it. */
export const SHELF_RETURN = {
  cardUnfoldMs: 500,
  cardUnfoldDelayMs: 100,
  rippleMs: 500,
  stripFlareMs: 1150,
  stripFlareDelayMs: 160,
} as const;

/**
 * §8.5.1's neighbour stagger: `0.16 + min(dist, 9) * 0.045` seconds.
 *
 * The clamp at 9 is the point of it — an unclamped distance on a 400-project shelf would put the
 * far corner's ripple 18 s after the near one, long past the 1600 ms at which the landing state is
 * cleared, so it would be removed mid-flight rather than played.
 */
export const RIPPLE_BASE_S = 0.16;
export const RIPPLE_STEP_S = 0.045;
export const RIPPLE_MAX_DISTANCE = 9;

export function rippleDelay(distance: number): string {
  const d = Math.min(Math.max(0, Math.floor(distance)), RIPPLE_MAX_DISTANCE);
  return `${(RIPPLE_BASE_S + d * RIPPLE_STEP_S).toFixed(3)}s`;
}

/**
 * Where the gesture is. `opening` and `closing` are the two windows in which the view on screen is
 * **not** the view the route asks for — which is the whole reason this exists rather than a
 * boolean.
 */
export type TransitionPhase =
  | { readonly kind: 'idle' }
  | { readonly kind: 'opening'; readonly id: ProjectId }
  | { readonly kind: 'closing'; readonly id: ProjectId }
  | { readonly kind: 'landing'; readonly id: ProjectId };

export const IDLE: TransitionPhase = { kind: 'idle' };

/**
 * What one tile does. `collapse` is the tile you opened; `unfold` is the tile you came back to;
 * `ripple` is every other tile in that tile's own section.
 */
export type CardGesture = 'collapse' | 'unfold' | 'ripple';

/**
 * §11.6: only `full` runs the gesture. Exported rather than inlined so the hook and the
 * stylesheet's own test ask the same question.
 */
export function gestureRuns(tier: ResolvedTier): boolean {
  return tier === 'full';
}

/**
 * How long the phase lasts before the machine advances, or `null` when it is terminal.
 *
 * `opening` ends when the view swaps; `closing` ends when the shelf is restored — both read from
 * `project/motion.ts`, because the page's own animation is timed against the same two instants.
 */
export function phaseDurationMs(phase: TransitionPhase): number | null {
  switch (phase.kind) {
    case 'opening':
      return PAGE_ENTRY.viewSwapMs;
    case 'closing':
      return PAGE_EXIT.shelfRestoredMs;
    case 'landing':
      return PAGE_EXIT.landingClearedMs;
    case 'idle':
      return null;
  }
}

/** The phase that follows, once {@link phaseDurationMs} has elapsed. */
export function nextPhase(phase: TransitionPhase): TransitionPhase {
  switch (phase.kind) {
    case 'opening':
      return IDLE;
    case 'closing':
      return { kind: 'landing', id: phase.id };
    case 'landing':
    case 'idle':
      return IDLE;
  }
}

/**
 * Which project the ROUTE should show, given the phase and what the caller asked for.
 *
 * During `opening` the shelf is still on screen collapsing, so the route must stay on the shelf
 * however eagerly the click set an id; during `closing` the page is still racking out, so the
 * route must stay on the page. Getting this backwards is not a visual defect — it unmounts the
 * element that is mid-animation, and the gesture becomes a hard cut with extra steps.
 */
export function routeProjectFor(phase: TransitionPhase, open: ProjectId | null): ProjectId | null {
  if (phase.kind === 'opening') return null;
  if (phase.kind === 'closing') return phase.id;
  return open;
}
