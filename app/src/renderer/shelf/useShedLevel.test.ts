import { act, renderHook } from '@testing-library/react';
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

let observed: (() => void) | null = null;
beforeEach(() => {
  observed = null;
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(cb: () => void) {
        observed = cb;
      }
      observe(): void {}
      disconnect(): void {}
    },
  );
});
afterEach(() => {
  vi.unstubAllGlobals();
});

function bar(width: number): HTMLDivElement {
  const el = document.createElement('div');
  Object.defineProperty(el, 'clientWidth', { value: width, configurable: true });
  document.body.append(el);
  return el;
}
function widen(el: HTMLElement, width: number): void {
  Object.defineProperty(el, 'clientWidth', { value: width, configurable: true });
}

describe('SHED_ORDER', () => {
  it("is §8.0a's order and nothing else", () => {
    expect([...SHED_ORDER]).toEqual(['switch', 'keys', 'wordmarkLettering']);
  });
  it('carries one threshold per step, descending', () => {
    expect(SHED_WIDTHS).toHaveLength(SHED_ORDER.length);
    expect([...SHED_WIDTHS]).toEqual([...SHED_WIDTHS].sort((a, b) => b - a));
  });
  it('carries one class per step, and they are the ones the stylesheet is asked for', () => {
    expect(SHED_CLASSES).toHaveLength(SHED_ORDER.length);
    expect([...SHED_CLASSES]).toEqual([
      'cdt-topbar--shed-1',
      'cdt-topbar--shed-2',
      'cdt-topbar--shed-3',
    ]);
  });
  it('names a class per step and none at rest', () => {
    expect(shedClassName(0)).toBeNull();
    expect(shedClassName(3)).toBe('cdt-topbar--shed-3');
  });
  it('never shares a threshold between two steps, which would skip one', () => {
    expect(new Set(SHED_WIDTHS).size).toBe(SHED_WIDTHS.length);
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
  it('holds each level across its whole band, not only at the threshold', () => {
    expect(shedLevelFor(SHED_WIDTHS[1] + 1)).toBe(1);
    expect(shedLevelFor(SHED_WIDTHS[2] + 1)).toBe(2);
  });
  it('stops at the last step — there is nothing further to drop', () => {
    expect(shedLevelFor(1)).toBe(3);
    expect(shedLevelFor(0)).toBe(3);
  });
});

describe('useShedLevel', () => {
  it('reads the live bar width and re-reads it on resize', () => {
    const ref = createRef<HTMLElement>();
    const el = bar(SHED_WIDTHS[0] + 200);
    (ref as { current: HTMLElement }).current = el;
    const { result } = renderHook(() => useShedLevel(ref));
    expect(result.current).toBe(0);

    act(() => {
      widen(el, SHED_WIDTHS[1]);
      observed?.();
    });
    expect(result.current).toBe(2);

    act(() => {
      widen(el, SHED_WIDTHS[0] + 200);
      observed?.();
    });
    expect(result.current).toBe(0);
  });

  it('sheds nothing when there is no bar to measure', () => {
    const ref = createRef<HTMLElement>();
    const { result } = renderHook(() => useShedLevel(ref));
    expect(result.current).toBe(0);
  });
});
