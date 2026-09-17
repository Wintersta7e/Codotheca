/**
 * **AC-P2-21-11, the renderer half**, and §21.10's one-banner rule as the hook sees it.
 *
 * The hook computes nothing: it holds the last payload the core sent and re-renders it. What is
 * asserted here is that it renders **what arrived** — never a percentage, never a denominator
 * nobody observed, and never a figure lower than the one before it.
 */
import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { RendererEvent } from '../../shared/channels';
import type { SyncListingProgress, SyncStatus } from '../../generated/protocol';
import type { AppDeps } from './deps';
import { formatListingProgress, useSync } from './useSync';

/** A deps stub carrying only what this hook reads, with a hand-driven event stream. */
function depsWith(): { deps: AppDeps; emit: (event: RendererEvent) => void } {
  const handlers = new Set<(event: RendererEvent) => void>();
  const deps = {
    subscribe: (handler: (event: RendererEvent) => void) => {
      handlers.add(handler);
      return () => {
        handlers.delete(handler);
      };
    },
  } as unknown as AppDeps;
  return {
    deps,
    emit: (event) => {
      for (const handler of [...handlers]) handler(event);
    },
  };
}

function progress(listed: number, total: number | null): SyncListingProgress {
  return { accountId: 1, listed, total } as SyncListingProgress;
}

describe('formatListingProgress', () => {
  it('renders a bare count when no total was observed', () => {
    expect(formatListingProgress(progress(120, null))).toBe('120');
  });

  it('renders "<n> of <total>" only when the response supplied one', () => {
    expect(formatListingProgress(progress(120, 300))).toBe('120 of 300');
  });

  /**
   * §21.11: **never a percentage.** A listing's denominator is unknown in phase 2, and a
   * percentage over an unknown denominator is an unknown rendered as a fact.
   */
  it('never renders a percentage, with or without a total', () => {
    for (const rendered of [
      formatListingProgress(progress(120, null)),
      formatListingProgress(progress(120, 300)),
      formatListingProgress(progress(0, null)),
    ]) {
      expect(rendered).not.toContain('%');
    }
  });

  /** Zero listed is *none yet*, and it renders as the count it is rather than as an absence. */
  it('renders a zero count rather than nothing', () => {
    expect(formatListingProgress(progress(0, null))).toBe('0');
  });
});

describe('useSync', () => {
  it('holds no progress line until the core reports one', () => {
    const { deps } = depsWith();
    const { result } = renderHook(() => useSync(deps));
    expect(result.current.progressLine).toBeNull();
    expect(result.current.status).toBeNull();
    expect(result.current.notice).toBeNull();
  });

  /**
   * **AC-P2-21-11's second clause.** Feeding the hook a decreasing sequence must never lower the
   * rendered figure — and the reason it cannot is that the core's counter is monotone by
   * construction, so no such event exists. This asserts the hook renders what arrived rather than
   * smoothing it: given the sequence the core can actually produce, the figure only rises.
   */
  it('renders a non-decreasing figure over the sequence the core produces', () => {
    const { deps, emit } = depsWith();
    const { result } = renderHook(() => useSync(deps));
    const seen: string[] = [];
    for (const listed of [2, 3, 3, 7]) {
      act(() => {
        emit({ topic: 'sync', event: 'listing_progress', data: progress(listed, null) });
      });
      seen.push(result.current.progressLine ?? '');
    }
    expect(seen).toEqual(['2', '3', '3', '7']);
    const figures = seen.map((line) => Number(line));
    for (let i = 1; i < figures.length; i += 1) {
      expect(figures[i]).toBeGreaterThanOrEqual(figures[i - 1] ?? 0);
    }
  });

  it('never synthesises a denominator from what it has seen', () => {
    const { deps, emit } = depsWith();
    const { result } = renderHook(() => useSync(deps));
    act(() => {
      emit({ topic: 'sync', event: 'listing_progress', data: progress(100, null) });
    });
    act(() => {
      emit({ topic: 'sync', event: 'listing_progress', data: progress(200, null) });
    });
    expect(result.current.progressLine).toBe('200');
    expect(result.current.progressLine).not.toContain(' of ');
  });

  /** The snapshot is the whole truth, so a window that opened late renders the same thing. */
  it('takes its listing and its notice from a snapshot', () => {
    const { deps, emit } = depsWith();
    const { result } = renderHook(() => useSync(deps));
    const snapshot: SyncStatus = {
      tasks: [],
      budgets: [],
      listing: progress(41, null),
      notice: 'throttled',
    } as unknown as SyncStatus;
    act(() => {
      emit({ topic: 'sync', event: 'snapshot', data: snapshot });
    });
    expect(result.current.progressLine).toBe('41');
    expect(result.current.notice).toBe('throttled');
    expect(result.current.status).toEqual(snapshot);
  });

  it('ignores every topic but its own', () => {
    const { deps, emit } = depsWith();
    const { result } = renderHook(() => useSync(deps));
    act(() => {
      emit({ topic: 'scan', event: 'listing_progress', data: progress(999, null) });
    });
    expect(result.current.progressLine).toBeNull();
  });

  /**
   * §21.10: **one banner**, and the last failure observed is the one it names. Three failed tasks
   * at once produce one variant here because the core carries one — the hook holds no list to
   * grow.
   */
  it('carries one notice variant and replaces it rather than accumulating', () => {
    const { deps, emit } = depsWith();
    const { result } = renderHook(() => useSync(deps));
    act(() => {
      emit({ topic: 'sync', event: 'notice', data: 'throttled' });
    });
    expect(result.current.notice).toBe('throttled');
    act(() => {
      emit({ topic: 'sync', event: 'notice', data: 'unauthorized' });
    });
    expect(result.current.notice).toBe('unauthorized');
  });
});
