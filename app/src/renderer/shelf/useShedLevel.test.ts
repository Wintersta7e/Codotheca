import { act, cleanup, renderHook } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  SHED_CLASSES,
  SHED_ORDER,
  SHED_WIDTHS,
  shedClassName,
  shedLevelFor,
  useShedLevel,
} from './useShedLevel.js';

afterEach(cleanup);

let observed: (() => void) | null = null;
let disconnects = 0;

beforeEach(() => {
  observed = null;
  disconnects = 0;
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(callback: () => void) {
        observed = callback;
      }
      observe(): void {
        // the fixture drives the callback directly
      }
      disconnect(): void {
        disconnects += 1;
      }
    },
  );
});

function bar(width: number): HTMLDivElement {
  const element = document.createElement('div');
  Object.defineProperty(element, 'clientWidth', { value: width, configurable: true });
  document.body.append(element);
  return element;
}

function widen(element: HTMLElement, width: number): void {
  Object.defineProperty(element, 'clientWidth', { value: width, configurable: true });
}

describe('SHED_ORDER', () => {
  it("is §8.0a's order and nothing else", () => {
    expect([...SHED_ORDER]).toEqual(['switch', 'keys', 'wordmarkLettering']);
  });
  it('carries one threshold per step, descending', () => {
    expect(SHED_WIDTHS).toHaveLength(SHED_ORDER.length);
    expect([...SHED_WIDTHS]).toEqual([...SHED_WIDTHS].sort((a, b) => b - a));
  });
  it('names a class per step and none at rest', () => {
    expect(shedClassName(0)).toBeNull();
    expect(shedClassName(1)).toBe('cdt-topbar--shed-1');
    expect(shedClassName(2)).toBe('cdt-topbar--shed-2');
    expect(shedClassName(3)).toBe('cdt-topbar--shed-3');
  });
  it('names one class per step and no more', () => {
    expect(SHED_CLASSES).toHaveLength(SHED_ORDER.length);
  });
});

describe('shedLevelFor', () => {
  it('sheds nothing above the first threshold', () => {
    expect(shedLevelFor(SHED_WIDTHS[0] + 1)).toBe(0);
  });
  it('sheds one step at each threshold, in order', () => {
    expect(shedLevelFor(SHED_WIDTHS[0])).toBe(1);
    expect(shedLevelFor(SHED_WIDTHS[1])).toBe(2);
    expect(shedLevelFor(SHED_WIDTHS[2])).toBe(3);
  });
  it('stops at the last step — there is nothing further to drop', () => {
    expect(shedLevelFor(1)).toBe(3);
  });
  it('never un-sheds as the bar narrows', () => {
    const widths = [2000, ...SHED_WIDTHS, 1];
    const levels = widths.map(shedLevelFor);
    expect(levels).toEqual([...levels].sort((a, b) => a - b));
  });
});

describe('useShedLevel', () => {
  it('reads the live bar width and re-reads it on resize', () => {
    const ref = createRef<HTMLElement>();
    const element = bar(SHED_WIDTHS[0] + 200);
    (ref as { current: HTMLElement }).current = element;
    const { result } = renderHook(() => useShedLevel(ref));
    expect(result.current).toBe(0);

    act(() => {
      widen(element, SHED_WIDTHS[1]);
      observed?.();
    });
    expect(result.current).toBe(2);

    act(() => {
      widen(element, SHED_WIDTHS[0] + 200);
      observed?.();
    });
    expect(result.current).toBe(0);
  });

  it('really observes — the fixture callback is the one the hook installed', () => {
    // Without this the two assertions above could both be satisfied by a hook that reads once
    // on mount and by a fixture callback nothing ever registered.
    const ref = createRef<HTMLElement>();
    (ref as { current: HTMLElement }).current = bar(2000);
    renderHook(() => useShedLevel(ref));
    expect(observed).not.toBeNull();
  });

  it('disconnects the observer when it unmounts', () => {
    const ref = createRef<HTMLElement>();
    (ref as { current: HTMLElement }).current = bar(2000);
    const { unmount } = renderHook(() => useShedLevel(ref));
    unmount();
    expect(disconnects).toBe(1);
  });

  it('sheds nothing when there is no element to measure', () => {
    const ref = createRef<HTMLElement>();
    const { result } = renderHook(() => useShedLevel(ref));
    expect(result.current).toBe(0);
    expect(observed).toBeNull();
  });

  it('reads the width even where ResizeObserver does not exist', () => {
    // The first read is not the observer's: a host without one still gets the right level on
    // mount rather than a bar that has shed everything.
    vi.stubGlobal('ResizeObserver', undefined);
    const ref = createRef<HTMLElement>();
    (ref as { current: HTMLElement }).current = bar(SHED_WIDTHS[2]);
    const { result } = renderHook(() => useShedLevel(ref));
    expect(result.current).toBe(3);
  });
});
