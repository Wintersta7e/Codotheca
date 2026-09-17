// @vitest-environment jsdom
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { PAGE_ENTRY, PAGE_EXIT } from '../project/motion';
import { useProjectTransition } from './useProjectTransition';
import type { ProjectId } from '../../generated/protocol';

const ALPHA = 1 as ProjectId;
const BETA = 2 as ProjectId;

/**
 * Plain fake timers, and deliberately **not** `{ shouldAdvanceTime: true }`.
 *
 * That option keeps the fake clock tracking real elapsed time, which turns every "not yet"
 * assertion below into a race against the machine: a stall of one millisecond between the advance
 * and the expectation lets the real clock finish the wait. It is the measured cause of the
 * rotating `FirstRunGate` failures, and this file asserts "one millisecond short" three times.
 */
afterEach(() => {
  vi.useRealTimers();
});

describe('§8.5.1: the view is held for the whole gesture', () => {
  it('keeps the shelf for the full 620 ms and swaps on the instant, not before', () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useProjectTransition('full'));

    act(() => {
      result.current.openProject(ALPHA);
    });
    expect(result.current.phase).toEqual({ kind: 'opening', id: ALPHA });
    expect(result.current.projectId).toBeNull();

    act(() => {
      vi.advanceTimersByTime(PAGE_ENTRY.viewSwapMs - 1);
    });
    expect(result.current.projectId).toBeNull();

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.projectId).toBe(ALPHA);
    expect(result.current.phase.kind).toBe('idle');
  });

  it('keeps the page racking out until the shelf is restored, then lands on the tile left', () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useProjectTransition('full'));

    act(() => {
      result.current.openProject(ALPHA);
    });
    act(() => {
      vi.advanceTimersByTime(PAGE_ENTRY.viewSwapMs);
    });

    act(() => {
      result.current.closeProject();
    });
    expect(result.current.phase).toEqual({ kind: 'closing', id: ALPHA });
    // The page is still on screen: `rackOut` runs for 340 ms and the shelf returns at 300 ms.
    expect(result.current.projectId).toBe(ALPHA);

    act(() => {
      vi.advanceTimersByTime(PAGE_EXIT.shelfRestoredMs - 1);
    });
    expect(result.current.projectId).toBe(ALPHA);

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.projectId).toBeNull();
    // The landing state names the tile to unfold and the section whose header flares.
    expect(result.current.phase).toEqual({ kind: 'landing', id: ALPHA });
  });

  it('clears the landing state at 1600 ms, so nothing inert is left on the shelf', () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useProjectTransition('full'));

    act(() => {
      result.current.openProject(ALPHA);
    });
    act(() => {
      vi.advanceTimersByTime(PAGE_ENTRY.viewSwapMs);
    });
    act(() => {
      result.current.closeProject();
    });
    act(() => {
      vi.advanceTimersByTime(PAGE_EXIT.shelfRestoredMs);
    });

    act(() => {
      vi.advanceTimersByTime(PAGE_EXIT.landingClearedMs - 1);
    });
    expect(result.current.phase.kind).toBe('landing');

    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(result.current.phase.kind).toBe('idle');
    expect(result.current.projectId).toBeNull();
  });
});

describe('§11.6: below `full` there is no gesture to wait for', () => {
  it('swaps immediately at reduced, with no phase and no timer', () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useProjectTransition('reduced'));

    act(() => {
      result.current.openProject(ALPHA);
    });
    expect(result.current.projectId).toBe(ALPHA);
    expect(result.current.phase.kind).toBe('idle');

    act(() => {
      result.current.closeProject();
    });
    expect(result.current.projectId).toBeNull();
    expect(result.current.phase.kind).toBe('idle');
  });

  it('swaps immediately at off too, so a clamped tier never waits out an animation it cannot play', () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useProjectTransition('off'));

    act(() => {
      result.current.openProject(ALPHA);
    });
    expect(result.current.projectId).toBe(ALPHA);
  });
});

describe('the gesture is shelf→project and nothing else', () => {
  /**
   * `opening` puts the shelf back on the route. Borrowing it for a link between two project pages
   * would flash the shelf for 620 ms on the way from one page to another — a gesture that says
   * "you went back" when nobody did.
   */
  it('does not run when following a link from one page to another', () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useProjectTransition('full'));

    act(() => {
      result.current.openProject(ALPHA);
    });
    act(() => {
      vi.advanceTimersByTime(PAGE_ENTRY.viewSwapMs);
    });
    expect(result.current.projectId).toBe(ALPHA);

    act(() => {
      result.current.openProject(BETA);
    });
    expect(result.current.projectId).toBe(BETA);
    expect(result.current.phase.kind).toBe('idle');
  });

  it('closing nothing does nothing, so a stray back press cannot start a gesture', () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useProjectTransition('full'));

    act(() => {
      result.current.closeProject();
    });
    expect(result.current.phase.kind).toBe('idle');
    expect(result.current.projectId).toBeNull();
  });
});
