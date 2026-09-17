import { describe, expect, it } from 'vitest';

import { PAGE_ENTRY, PAGE_EXIT } from '../project/motion';
import {
  IDLE,
  RIPPLE_BASE_S,
  RIPPLE_MAX_DISTANCE,
  RIPPLE_STEP_S,
  gestureRuns,
  nextPhase,
  phaseDurationMs,
  rippleDelay,
  routeProjectFor,
  type TransitionPhase,
} from './transition';
import type { ProjectId } from '../../generated/protocol';

const ALPHA = 1 as ProjectId;
const BETA = 2 as ProjectId;

describe('§8.5.1: the neighbour stagger', () => {
  it('is 0.16 s at the tile itself and steps by 0.045 s', () => {
    expect(rippleDelay(0)).toBe(`${RIPPLE_BASE_S.toFixed(3)}s`);
    expect(rippleDelay(1)).toBe(`${(RIPPLE_BASE_S + RIPPLE_STEP_S).toFixed(3)}s`);
    expect(rippleDelay(4)).toBe(`${(RIPPLE_BASE_S + 4 * RIPPLE_STEP_S).toFixed(3)}s`);
  });

  /**
   * The clamp is the point of the rule, not a guard on it. §8.5.1 writes `min(dist, 9)`, and the
   * landing state is cleared at 1600 ms: an unclamped distance on a large section would schedule a
   * ripple after the element carrying it has already been taken away, so it would never play.
   */
  it('clamps at nine, so no ripple is scheduled past the landing state', () => {
    const capped = rippleDelay(RIPPLE_MAX_DISTANCE);
    expect(rippleDelay(RIPPLE_MAX_DISTANCE + 1)).toBe(capped);
    expect(rippleDelay(400)).toBe(capped);
    const cappedMs = (RIPPLE_BASE_S + RIPPLE_MAX_DISTANCE * RIPPLE_STEP_S) * 1000;
    expect(cappedMs).toBeLessThan(PAGE_EXIT.landingClearedMs);
  });

  it('treats a negative distance as the tile itself rather than reversing the stagger', () => {
    expect(rippleDelay(-3)).toBe(rippleDelay(0));
  });
});

describe('§11.6: the gesture is `full` only', () => {
  it('does not run at reduced or off, and there is no clamped version', () => {
    expect(gestureRuns('full')).toBe(true);
    expect(gestureRuns('reduced')).toBe(false);
    expect(gestureRuns('off')).toBe(false);
  });
});

describe('the phase machine', () => {
  it('takes its two hand-off instants from the page, never restating them', () => {
    expect(phaseDurationMs({ kind: 'opening', id: ALPHA })).toBe(PAGE_ENTRY.viewSwapMs);
    expect(phaseDurationMs({ kind: 'closing', id: ALPHA })).toBe(PAGE_EXIT.shelfRestoredMs);
    expect(phaseDurationMs({ kind: 'landing', id: ALPHA })).toBe(PAGE_EXIT.landingClearedMs);
  });

  it('is terminal at idle, so nothing is scheduled while the shelf is at rest', () => {
    expect(phaseDurationMs(IDLE)).toBeNull();
  });

  it('runs closing into landing, and everything else into idle', () => {
    expect(nextPhase({ kind: 'closing', id: ALPHA })).toEqual({ kind: 'landing', id: ALPHA });
    expect(nextPhase({ kind: 'opening', id: ALPHA })).toEqual(IDLE);
    expect(nextPhase({ kind: 'landing', id: ALPHA })).toEqual(IDLE);
    expect(nextPhase(IDLE)).toEqual(IDLE);
  });
});

/**
 * The route is the half of this that has a wrong answer rather than an ugly one. Showing the page
 * during `opening` unmounts the shelf while `crtCollapse`, `shelfRecede` and the beam are still
 * running on it; showing the shelf during `closing` does the same to `rackOut`. Either way the
 * gesture becomes a hard cut that takes exactly as long as the gesture would have.
 */
describe('which view the route shows', () => {
  it('holds the shelf for the whole of opening, however eagerly the click set an id', () => {
    expect(routeProjectFor({ kind: 'opening', id: ALPHA }, ALPHA)).toBeNull();
    expect(routeProjectFor({ kind: 'opening', id: ALPHA }, null)).toBeNull();
  });

  it('holds the page for the whole of closing, and names the page being left', () => {
    expect(routeProjectFor({ kind: 'closing', id: ALPHA }, null)).toBe(ALPHA);
    // The phase wins over a stale id: the page racking out is the one being closed.
    expect(routeProjectFor({ kind: 'closing', id: ALPHA }, BETA)).toBe(ALPHA);
  });

  it('defers to the caller at idle and while landing, because the shelf is already on screen', () => {
    const landing: TransitionPhase = { kind: 'landing', id: ALPHA };
    expect(routeProjectFor(landing, null)).toBeNull();
    expect(routeProjectFor(IDLE, BETA)).toBe(BETA);
    expect(routeProjectFor(IDLE, null)).toBeNull();
  });
});
