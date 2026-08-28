import { describe, expect, it } from 'vitest';
import { COALESCE_WINDOW_MS, createCoalescer } from './coalesce';

interface FakeClock {
  run(): void;
  schedule: (fn: () => void, ms: number) => () => void;
  readonly lastMs: number | null;
}

function fakeClock(): FakeClock {
  let pending: (() => void) | null = null;
  const state = { lastMs: null as number | null };
  return {
    get lastMs(): number | null {
      return state.lastMs;
    },
    schedule: (fn, ms) => {
      pending = fn;
      state.lastMs = ms;
      return (): void => {
        pending = null;
      };
    },
    run: () => {
      const f = pending;
      pending = null;
      f?.();
    },
  };
}

describe('coalescer', () => {
  it('turns a thousand events into one batch, not a thousand messages', () => {
    const clock = fakeClock();
    const batches: number[][] = [];
    const c = createCoalescer<number>({
      windowMs: COALESCE_WINDOW_MS,
      sink: (b) => batches.push(b),
      schedule: clock.schedule,
    });
    for (let i = 0; i < 1000; i += 1) c.push(i);
    expect(batches.length).toBe(0);
    expect(clock.lastMs).toBe(COALESCE_WINDOW_MS);
    clock.run();
    expect(batches.length).toBe(1);
    expect(batches[0]?.length).toBe(1000);
  });

  it('sends nothing at all for an empty window', () => {
    const clock = fakeClock();
    const batches: number[][] = [];
    const c = createCoalescer<number>({
      windowMs: 16,
      sink: (b) => batches.push(b),
      schedule: clock.schedule,
    });
    c.flushNow();
    expect(batches.length).toBe(0);
    c.push(1);
    c.flushNow();
    expect(batches).toEqual([[1]]);
  });

  it('drops what was buffered and cancels the window on stop', () => {
    const clock = fakeClock();
    const batches: number[][] = [];
    const c = createCoalescer<number>({
      windowMs: 16,
      sink: (b) => batches.push(b),
      schedule: clock.schedule,
    });
    c.push(1);
    c.stop();
    clock.run();
    expect(batches.length).toBe(0);
  });
});
