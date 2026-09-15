import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId } from '../../generated/protocol';
import {
  FLICKER_DEPTH_RANGE,
  FLICKER_DIP_COUNT_RANGE,
  FLICKER_HOLD_MS_RANGE,
  FLICKER_MEAN_MS,
  FLICKER_OPACITY_PROPERTY,
  type FlickerRow,
  flickerEligible,
  flickerRandom,
  nextFlickerEvent,
  useFlicker,
} from './flicker';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const row = (over: Partial<FlickerRow> = {}): FlickerRow => ({
  conditionSignal: 'neglected',
  isReference: false,
  presence: 'present',
  ...over,
});

describe('§11.6 eligibility, which the old acceptance wording omitted entirely', () => {
  it('admits neglected and abandoned, present, and not reference', () => {
    expect(flickerEligible(row())).toBe(true);
    expect(flickerEligible(row({ conditionSignal: 'abandoned' }))).toBe(true);
  });

  it('rejects every other band, so the whole shelf is not a candidate', () => {
    for (const signal of ['live', 'idle', 'dormant', 'offline', 'empty'] as const) {
      expect(flickerEligible(row({ conditionSignal: signal }))).toBe(false);
    }
    expect(flickerEligible(row({ conditionSignal: null }))).toBe(false);
  });

  it('rejects a reference project and a location that is not present', () => {
    expect(flickerEligible(row({ isReference: true }))).toBe(false);
    expect(flickerEligible(row({ presence: 'offline' }))).toBe(false);
    expect(flickerEligible(row({ presence: 'missing' }))).toBe(false);
    expect(flickerEligible(row({ presence: 'unscanned' }))).toBe(false);
  });

  // §23.2: `presence` becomes nullable on the row and a not-cloned project carries `null`.
  // `presence !== 'present'` is already correct under it, so this is an assertion and not a
  // change — the design says the same thing: "Only genuinely absent things (blueprint,
  // reference) sit at zero".
  it('rejects a project with no working copy at all', () => {
    expect(flickerEligible(row({ presence: null }))).toBe(false);
    expect(flickerEligible(row({ presence: null, conditionSignal: 'abandoned' }))).toBe(false);
  });
});

describe('the schedule carries §11.6‘s parameters and not the design‘s', () => {
  it('is a Poisson process at a ~9 s mean, not a fixed 9,000 ms period', () => {
    const rand = flickerRandom(7);
    const gaps: number[] = [];
    let at = 0;
    for (let i = 0; i < 400; i += 1) {
      const event = nextFlickerEvent(rand, at, 3);
      const first = event.steps[0];
      if (first === undefined) throw new Error('no steps');
      gaps.push(first.atMs - at);
      at = event.endMs;
    }
    const mean = gaps.reduce((s, g) => s + g, 0) / gaps.length;
    expect(mean).toBeGreaterThan(FLICKER_MEAN_MS * 0.75);
    expect(mean).toBeLessThan(FLICKER_MEAN_MS * 1.25);
    expect(new Set(gaps).size).toBeGreaterThan(100);
  });

  it('dips 1–2 times, 15–25% deep, holding 80–140 ms, and always returns to steady', () => {
    const rand = flickerRandom(11);
    let at = 0;
    for (let i = 0; i < 200; i += 1) {
      const event = nextFlickerEvent(rand, at, 5);
      const dips = event.steps.filter((s) => s.opacity < 1);
      expect(dips.length).toBeGreaterThanOrEqual(FLICKER_DIP_COUNT_RANGE[0]);
      expect(dips.length).toBeLessThanOrEqual(FLICKER_DIP_COUNT_RANGE[1]);
      for (const dip of dips) {
        expect(1 - dip.opacity).toBeGreaterThanOrEqual(FLICKER_DEPTH_RANGE[0] - 1e-9);
        expect(1 - dip.opacity).toBeLessThanOrEqual(FLICKER_DEPTH_RANGE[1] + 1e-9);
      }
      const last = event.steps.at(-1);
      expect(last?.opacity).toBe(1);
      const holds = dips.map((dip, n) => {
        const back = event.steps[event.steps.indexOf(dip) + 1];
        if (back === undefined) throw new Error(`no return for dip ${String(n)}`);
        return back.atMs - dip.atMs;
      });
      for (const hold of holds) {
        expect(hold).toBeGreaterThanOrEqual(FLICKER_HOLD_MS_RANGE[0]);
        expect(hold).toBeLessThanOrEqual(FLICKER_HOLD_MS_RANGE[1]);
      }
      at = event.endMs;
    }
  });

  it('never carries the design‘s cut parameters', () => {
    const rand = flickerRandom(3);
    const event = nextFlickerEvent(rand, 0, 4);
    for (const step of event.steps) {
      expect(step.opacity).not.toBe(0.5);
    }
  });

  it('is a dip from steady and never a brighten from dark', () => {
    const rand = flickerRandom(5);
    for (let i = 0, at = 0; i < 100; i += 1) {
      const event = nextFlickerEvent(rand, at, 2);
      for (const step of event.steps) expect(step.opacity).toBeLessThanOrEqual(1);
      at = event.endMs;
    }
  });

  it('is deterministic for a seed, so a run is reproducible', () => {
    const a = nextFlickerEvent(flickerRandom(42), 0, 6);
    const b = nextFlickerEvent(flickerRandom(42), 0, 6);
    expect(a).toStrictEqual(b);
  });

  it('picks exactly one candidate per event', () => {
    const rand = flickerRandom(9);
    const event = nextFlickerEvent(rand, 0, 3);
    expect(event.projectIndex).toBeGreaterThanOrEqual(0);
    expect(event.projectIndex).toBeLessThan(3);
  });
});

/**
 * The driver. Every assertion below is criterion 21's — the parameters above are only numbers
 * until something plays them, and a schedule that emits its dip and its return in the same tick
 * satisfies every one of the pure tests while producing no visible flicker at all.
 */
const ids = [11, 22, 33].map((n) => n as unknown as ProjectId);

interface Harness {
  readonly clock: { value: number };
  readonly monotonicMs: () => number;
}

function harness(): Harness {
  const clock = { value: 0 };
  return { clock, monotonicMs: (): number => clock.value };
}

/** Advances the fake timers and the monotonic clock together; they are the same clock. */
function advance(h: Harness, ms: number): void {
  act(() => {
    h.clock.value += ms;
    vi.advanceTimersByTime(ms);
  });
}

describe('useFlicker: at most one card dips, and only when the tier and the window allow it', () => {
  it('schedules nothing at reduced and nothing at off', () => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    for (const tier of ['reduced', 'off'] as const) {
      const h = harness();
      const spy = vi.spyOn(globalThis, 'setTimeout');
      const view = renderHook(() => useFlicker(ids, { tier, seed: 5, monotonicMs: h.monotonicMs }));
      expect(spy).not.toHaveBeenCalled();
      expect(view.result.current).toStrictEqual({ projectId: null, opacity: 1 });
      spy.mockRestore();
      view.unmount();
    }
  });

  it('schedules nothing while the window is unfocused, and nothing with no candidates', () => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const h = harness();
    const blurred = vi.spyOn(globalThis, 'setTimeout');
    const view = renderHook(() =>
      useFlicker(ids, { tier: 'full', seed: 5, monotonicMs: h.monotonicMs }),
    );
    expect(blurred).not.toHaveBeenCalled();
    view.unmount();
    blurred.mockRestore();

    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const empty = vi.spyOn(globalThis, 'setTimeout');
    const none = renderHook(() =>
      useFlicker([], { tier: 'full', seed: 5, monotonicMs: h.monotonicMs }),
    );
    expect(empty).not.toHaveBeenCalled();
    expect(none.result.current.projectId).toBeNull();
  });

  it('dips one card at the scheduled moment and returns it to steady', () => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const h = harness();
    // The same event the hook will play, computed here from the same seed and the same count.
    const event = nextFlickerEvent(flickerRandom(5), 0, ids.length);
    const dip = event.steps[0];
    const back = event.steps[1];
    if (dip === undefined || back === undefined) throw new Error('no dip');

    const view = renderHook(() =>
      useFlicker(ids, { tier: 'full', seed: 5, monotonicMs: h.monotonicMs }),
    );
    expect(view.result.current).toStrictEqual({ projectId: null, opacity: 1 });

    advance(h, dip.atMs);
    expect(view.result.current.projectId).toBe(ids[event.projectIndex]);
    expect(view.result.current.opacity).toBe(dip.opacity);
  });

  it('holds the dip for its whole 80–140 ms and does not return in the same tick', () => {
    // The bug this catches emits every step back to back at zero delay: each pure parameter
    // test still passes, the dip and its return both happen inside one timer callback, and
    // nothing flickers on screen. The hold is the effect.
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const h = harness();
    const event = nextFlickerEvent(flickerRandom(5), 0, ids.length);
    const dip = event.steps[0];
    const back = event.steps[1];
    if (dip === undefined || back === undefined) throw new Error('no dip');
    const hold = back.atMs - dip.atMs;
    expect(hold).toBeGreaterThanOrEqual(FLICKER_HOLD_MS_RANGE[0]);

    const view = renderHook(() =>
      useFlicker(ids, { tier: 'full', seed: 5, monotonicMs: h.monotonicMs }),
    );
    advance(h, dip.atMs);
    expect(view.result.current.opacity).toBe(dip.opacity);

    advance(h, Math.floor(hold) - 1);
    expect(view.result.current.opacity).toBe(dip.opacity);

    advance(h, 2);
    expect(view.result.current.opacity).toBe(1);
  });

  it('is not restarted by a new candidate array on every render', () => {
    // Plan 13 recomputes `visibleProjectIds` per scroll frame. A schedule keyed on the array's
    // identity reseeds its ~9 s gap on every one of those renders and never emits a dip.
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const h = harness();
    const event = nextFlickerEvent(flickerRandom(5), 0, ids.length);
    const dip = event.steps[0];
    if (dip === undefined) throw new Error('no dip');

    const view = renderHook(() =>
      useFlicker([...ids], { tier: 'full', seed: 5, monotonicMs: h.monotonicMs }),
    );
    for (let i = 0; i < 6; i += 1) {
      advance(h, Math.floor(dip.atMs / 8));
      view.rerender();
    }
    advance(h, dip.atMs);
    expect(view.result.current.projectId).not.toBeNull();
  });

  it('returns to steady and clears its timer when the tier drops mid-dip', () => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const h = harness();
    const event = nextFlickerEvent(flickerRandom(5), 0, ids.length);
    const dip = event.steps[0];
    if (dip === undefined) throw new Error('no dip');

    let tier: 'full' | 'off' = 'full';
    const view = renderHook(() => useFlicker(ids, { tier, seed: 5, monotonicMs: h.monotonicMs }));
    advance(h, dip.atMs);
    expect(view.result.current.opacity).toBe(dip.opacity);

    tier = 'off';
    act(() => {
      view.rerender();
    });
    expect(view.result.current).toStrictEqual({ projectId: null, opacity: 1 });
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('the dip is a custom property, never an inline opacity', () => {
  it('names the property card.css reads, because an inline value outranks the tier clamp', () => {
    expect(FLICKER_OPACITY_PROPERTY).toBe('--cdt-halo-opacity');
    expect(FLICKER_OPACITY_PROPERTY.startsWith('--')).toBe(true);
  });
});
