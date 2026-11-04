/**
 * §43.15's one read: `motivation.get` once on mount, then the `motivation` topic's events and
 * nothing else. The hook schedules nothing — a poll here would be a second change signal beside
 * the core's one publisher.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { MotivationState } from '../../generated/protocol';
import type { RendererEvent } from '../../shared/channels';
import type { AppDeps } from '../app/deps';
import { useMotivation } from './useMotivation';

afterEach(() => {
  vi.useRealTimers();
});

const state = (over: Partial<MotivationState> = {}): MotivationState => ({
  shown: true,
  level: null,
  rings: null,
  xpBar: null,
  pendingLine: null,
  terms: null,
  ...over,
});

interface Harness {
  deps: Pick<AppDeps, 'request' | 'subscribe'>;
  request: ReturnType<typeof vi.fn>;
  emit: (event: RendererEvent) => void;
  resolve: (value: MotivationState) => void;
  reject: () => void;
}

function harness(): Harness {
  const handlers = new Set<(event: RendererEvent) => void>();
  let resolve: (value: MotivationState) => void = () => undefined;
  let reject: () => void = () => undefined;
  const request = vi.fn(
    () =>
      new Promise<MotivationState>((ok, fail) => {
        resolve = ok;
        reject = () => {
          fail(new Error('refused'));
        };
      }),
  );
  const deps = {
    request,
    subscribe: (handler: (event: RendererEvent) => void) => {
      handlers.add(handler);
      return () => {
        handlers.delete(handler);
      };
    },
  } as unknown as Pick<AppDeps, 'request' | 'subscribe'>;
  return {
    deps,
    request,
    emit: (event) => {
      for (const handler of [...handlers]) handler(event);
    },
    resolve: (value) => {
      resolve(value);
    },
    reject: () => {
      reject();
    },
  };
}

describe('useMotivation', () => {
  it('reads motivation.get once across rerenders, and holds its answer', async () => {
    const h = harness();
    const { result, rerender } = renderHook(() => useMotivation(h.deps));
    rerender();
    rerender();
    expect(h.request).toHaveBeenCalledTimes(1);
    expect(h.request).toHaveBeenCalledWith('motivation.get', {});
    expect(result.current).toBeNull();
    await act(async () => {
      h.resolve(state({ xpBar: 0.5 }));
      await Promise.resolve();
    });
    expect(result.current?.xpBar).toBe(0.5);
  });

  it('leaves null after a failed read — never a zeroed state', async () => {
    const h = harness();
    const { result } = renderHook(() => useMotivation(h.deps));
    await act(async () => {
      h.reject();
      await Promise.resolve();
    });
    expect(result.current).toBeNull();
  });

  it('replaces the state with a changed event', () => {
    const h = harness();
    const { result } = renderHook(() => useMotivation(h.deps));
    act(() => {
      h.emit({ topic: 'motivation', event: 'changed', data: state({ shown: false }) });
    });
    expect(result.current?.shown).toBe(false);
  });

  it('replaces the state with a snapshot event', () => {
    const h = harness();
    const { result } = renderHook(() => useMotivation(h.deps));
    act(() => {
      h.emit({ topic: 'motivation', event: 'changed', data: state({ xpBar: 0.1 }) });
    });
    act(() => {
      h.emit({ topic: 'motivation', event: 'snapshot', data: state({ xpBar: 0.9 }) });
    });
    expect(result.current?.xpBar).toBe(0.9);
  });

  it('ignores another topic, and a payload that is not a motivation state', () => {
    const h = harness();
    const { result } = renderHook(() => useMotivation(h.deps));
    act(() => {
      h.emit({ topic: 'sync', event: 'snapshot', data: state() });
      h.emit({ topic: 'motivation', event: 'changed', data: { nothing: true } });
    });
    expect(result.current).toBeNull();
  });

  it('lets no read answer overwrite an event that arrived after the read was sent', async () => {
    const h = harness();
    const { result } = renderHook(() => useMotivation(h.deps));
    act(() => {
      h.emit({ topic: 'motivation', event: 'changed', data: state({ xpBar: 0.8 }) });
    });
    await act(async () => {
      h.resolve(state({ xpBar: 0.2 }));
      await Promise.resolve();
    });
    expect(result.current?.xpBar).toBe(0.8);
  });

  it('schedules nothing: no timer and no second read over an hour', () => {
    vi.useFakeTimers();
    const interval = vi.spyOn(globalThis, 'setInterval');
    const timeout = vi.spyOn(globalThis, 'setTimeout');
    const h = harness();
    const before = { interval: interval.mock.calls.length, timeout: timeout.mock.calls.length };
    renderHook(() => useMotivation(h.deps));
    act(() => {
      vi.advanceTimersByTime(60 * 60 * 1000);
    });
    expect(h.request).toHaveBeenCalledTimes(1);
    expect(interval.mock.calls.length).toBe(before.interval);
    expect(timeout.mock.calls.length).toBe(before.timeout);
  });
});
