import { act, renderHook } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId } from '../../generated/protocol.js';
import { parseCollapseState } from './collapse.js';
import { EMPTY_MOUNT_WINDOW, sectionExtents, SECTION_HEADER_HEIGHT } from './measure.js';
import type { ShelfPage } from './page.js';
import { applyPeekHeight, rowTopOf, sameWindow, useVirtualizer } from './useVirtualizer.js';
import { noop } from '../noop.js';

function fakePage(counts: readonly number[]): ShelfPage {
  let next = 1;
  const sections = counts.map((count, index) => ({
    id: `era:${String(2020 - index)}`,
    order: 10 + index,
    year: 2020 - index,
    cutAgainstYear: 2026,
    label: String(2020 - index),
    agg: {
      count,
      trackedBytes: 0,
      indexedCount: count,
      unpushed: 0,
      uncommitted: 0,
      interrupted: 0,
      unchecked: 0,
    },
    rows: Array.from({ length: count }, () => ({ id: next++ })),
  }));
  const total = counts.reduce((a, b) => a + b, 0);
  return {
    sections,
    reference: [],
    ignored: [],
    matched: total,
    renderedTotal: total,
    orderKey: 'k',
    generation: 1,
    ast: { grammarVersion: 1, terms: [], ignored: [] },
  } as unknown as ShelfPage;
}

/**
 * Every fixture below is larger than §8.1's 150-item auto-collapse threshold and sits at order
 * 10, so a bare collapse state closes it and every window assertion would pass over an empty
 * section. The explicit expansion is what keeps these tests about virtualization.
 */
const open = parseCollapseState(['!era:2020']);

/** jsdom has no ResizeObserver and no layout; both are injected rather than assumed. */
let observed: (() => void) | null = null;
/** Frames the hook has scheduled and not yet run. Emptied by `flushFrames`. */
let frames: (() => void)[] = [];

beforeEach(() => {
  observed = null;
  frames = [];
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(cb: () => void) {
        observed = cb;
      }
      readonly observe = noop;
      readonly disconnect = noop;
    },
  );
  // jsdom's own rAF is timer-driven, so `act()` returns before the callback runs and every
  // scroll assertion below would read a window the hook had not committed yet.
  vi.stubGlobal('requestAnimationFrame', (cb: () => void): number => {
    frames.push(cb);
    return frames.length;
  });
  vi.stubGlobal('cancelAnimationFrame', (handle: number): void => {
    frames[handle - 1] = noop;
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
});

function flushFrames(): void {
  const pending = frames;
  frames = [];
  for (const frame of pending) frame();
}

function container(width: number, height: number): HTMLDivElement {
  const el = document.createElement('div');
  Object.defineProperty(el, 'clientWidth', { value: width, configurable: true });
  Object.defineProperty(el, 'clientHeight', { value: height, configurable: true });
  el.scrollTop = 0;
  document.body.append(el);
  return el;
}

function scrollTo(el: HTMLElement, top: number): void {
  act(() => {
    el.scrollTop = top;
    el.dispatchEvent(new Event('scroll'));
    flushFrames();
  });
}

describe('sameWindow', () => {
  it('is the commit point: equal windows must not re-render', () => {
    expect(sameWindow({ from: 0, to: 40 }, { from: 0, to: 40 })).toBe(true);
    expect(sameWindow({ from: 0, to: 40 }, { from: 4, to: 40 })).toBe(false);
    expect(sameWindow({ from: 0, to: 40 }, { from: 0, to: 41 })).toBe(false);
  });
});

describe('useVirtualizer', () => {
  const mount = (
    over: Partial<Parameters<typeof useVirtualizer>[0]> = {},
  ): { el: HTMLDivElement; result: { current: ReturnType<typeof useVirtualizer> } } => {
    const ref = createRef<HTMLElement>();
    const el = container(1290, 900);
    (ref as { current: HTMLElement }).current = el;
    const { result } = renderHook(() =>
      useVirtualizer({
        page: fakePage([1000]),
        tile: 186,
        collapse: open,
        queryIsEmpty: true,
        focusedProjectId: null,
        peekExtra: null,
        scrollRef: ref,
        ...over,
      }),
    );
    act(() => {
      flushFrames();
    });
    return { el, result };
  };

  it('opens the fixture section, so no window assertion passes on an empty one', () => {
    const { result } = mount();
    expect(result.current.extents[0]?.collapsed).toBe(false);
  });

  it('measures the container and mounts only a window of a 1,000-row page', () => {
    const { result } = mount();
    expect(result.current.metrics.columns).toBe(6);
    expect(result.current.canvasHeight).toBeGreaterThan(10_000);
    expect(result.current.mountWindow.to - result.current.mountWindow.from).toBeLessThan(60);
    expect(result.current.mountWindow.to).toBeGreaterThan(0);
  });

  it('does not re-render when a scroll leaves the window where it was', () => {
    const ref = createRef<HTMLElement>();
    const el = container(1290, 900);
    (ref as { current: HTMLElement }).current = el;
    let renders = 0;
    const { result } = renderHook(() => {
      renders += 1;
      return useVirtualizer({
        page: fakePage([1000]),
        tile: 186,
        collapse: open,
        queryIsEmpty: true,
        focusedProjectId: null,
        peekExtra: null,
        scrollRef: ref,
      });
    });
    act(() => {
      flushFrames();
    });
    const before = renders;
    const first = result.current.mountWindow;
    expect(first.to).toBeGreaterThan(0);

    scrollTo(el, 1);
    expect(renders).toBe(before);
    expect(result.current.mountWindow).toBe(first);

    scrollTo(el, 6000);
    expect(renders).toBeGreaterThan(before);
    expect(result.current.mountWindow.from).toBeGreaterThan(0);
  });

  it('coalesces a burst of scroll events into one frame', () => {
    // A wheel gesture fires far more scroll events than frames; a handler that recomputes per
    // event is the shelf's only realistic way to miss one.
    const ref = createRef<HTMLElement>();
    const el = container(1290, 900);
    (ref as { current: HTMLElement }).current = el;
    renderHook(() =>
      useVirtualizer({
        page: fakePage([1000]),
        tile: 186,
        collapse: open,
        queryIsEmpty: true,
        focusedProjectId: null,
        peekExtra: null,
        scrollRef: ref,
      }),
    );
    act(() => {
      flushFrames();
    });
    act(() => {
      for (let i = 1; i <= 12; i += 1) {
        el.scrollTop = i * 500;
        el.dispatchEvent(new Event('scroll'));
      }
    });
    expect(frames).toHaveLength(1);
  });

  it('re-measures when the container resizes', () => {
    const ref = createRef<HTMLElement>();
    const el = container(1290, 900);
    (ref as { current: HTMLElement }).current = el;
    const { result } = renderHook(() =>
      useVirtualizer({
        page: fakePage([200]),
        tile: 186,
        collapse: open,
        queryIsEmpty: true,
        focusedProjectId: null,
        peekExtra: null,
        scrollRef: ref,
      }),
    );
    expect(result.current.metrics.columns).toBe(6);
    act(() => {
      Object.defineProperty(el, 'clientWidth', { value: 700, configurable: true });
      observed?.();
      flushFrames();
    });
    expect(result.current.metrics.columns).toBe(3);
  });

  it('re-anchors a density change on the focused project, never the pixel offset', () => {
    const ref = createRef<HTMLElement>();
    const el = container(1290, 900);
    (ref as { current: HTMLElement }).current = el;
    const page = fakePage([300]);
    const { rerender, result } = renderHook(
      ({ tile }: { tile: number }) =>
        useVirtualizer({
          page,
          tile,
          collapse: open,
          queryIsEmpty: true,
          focusedProjectId: 120 as unknown as ProjectId,
          peekExtra: null,
          scrollRef: ref,
        }),
      { initialProps: { tile: 186 } },
    );
    act(() => {
      flushFrames();
    });
    expect(result.current.extents[0]?.collapsed).toBe(false);
    scrollTo(el, 5000);
    const before = el.scrollTop;
    expect(before).toBe(5000);

    act(() => {
      rerender({ tile: 232 });
      flushFrames();
    });
    expect(el.scrollTop).not.toBe(before);
    // The landing is the focused project's own row under the *new* column count — not the old
    // pixel offset, which addresses a different row once the grid re-cuts (§8.0a).
    const metrics = result.current.metrics;
    const top = rowTopOf(result.current.extents, metrics, 119);
    expect(top).not.toBeNull();
    expect(el.scrollTop).toBe(Math.max(0, (top ?? 0) - 900 / 2));
  });

  it('leaves the offset alone when nothing is focused — there is nothing to anchor on', () => {
    const ref = createRef<HTMLElement>();
    const el = container(1290, 900);
    (ref as { current: HTMLElement }).current = el;
    const page = fakePage([300]);
    const { rerender } = renderHook(
      ({ tile }: { tile: number }) =>
        useVirtualizer({
          page,
          tile,
          collapse: open,
          queryIsEmpty: true,
          focusedProjectId: null,
          peekExtra: null,
          scrollRef: ref,
        }),
      { initialProps: { tile: 186 } },
    );
    act(() => {
      flushFrames();
    });
    scrollTo(el, 5000);
    act(() => {
      rerender({ tile: 232 });
      flushFrames();
    });
    expect(el.scrollTop).toBe(5000);
  });

  it('mounts nothing, rather than the row at index 0, when every section is collapsed', () => {
    const { result } = mount({ collapse: parseCollapseState(['era:2020']) });
    expect(result.current.mountWindow).toEqual(EMPTY_MOUNT_WINDOW);
  });
});

describe('applyPeekHeight', () => {
  const metrics = { columns: 4, trackWidth: 200, rowHeight: 300 };
  it('grows the section Peek opened in, and pushes every later section down', () => {
    const page = fakePage([8, 8]);
    const base = sectionExtents(page, metrics, parseCollapseState([]), true);
    const grown = applyPeekHeight(base.extents, base.canvasHeight, 'era:2020', 140);
    expect(grown.extents[0]?.bodyHeight).toBe((base.extents[0]?.bodyHeight ?? 0) + 140);
    expect(grown.extents[1]?.top).toBe((base.extents[1]?.top ?? 0) + 140);
    expect(grown.canvasHeight).toBe(base.canvasHeight + 140);
  });
  it('leaves the section above Peek exactly where it was', () => {
    const page = fakePage([8, 8]);
    const base = sectionExtents(page, metrics, parseCollapseState([]), true);
    const grown = applyPeekHeight(base.extents, base.canvasHeight, 'era:2019', 140);
    expect(grown.extents[0]).toBe(base.extents[0]);
    expect(grown.extents[1]?.bodyHeight).toBe((base.extents[1]?.bodyHeight ?? 0) + 140);
  });
  it('is identity for a height of zero, so a closed Peek costs nothing', () => {
    const page = fakePage([8]);
    const base = sectionExtents(page, metrics, parseCollapseState([]), true);
    expect(applyPeekHeight(base.extents, base.canvasHeight, 'era:2020', 0).extents).toBe(
      base.extents,
    );
  });
});

describe('rowTopOf', () => {
  const metrics = { columns: 4, trackWidth: 200, rowHeight: 300 };
  it('locates a flat index so a re-anchor has somewhere to scroll to', () => {
    const page = fakePage([8, 8]);
    const { extents } = sectionExtents(page, metrics, parseCollapseState([]), true);
    expect(rowTopOf(extents, metrics, 0)).toBeGreaterThan(SECTION_HEADER_HEIGHT - 1);
    const twelve = rowTopOf(extents, metrics, 12);
    const zero = rowTopOf(extents, metrics, 0);
    expect(twelve).not.toBeNull();
    expect(zero).not.toBeNull();
    expect(twelve ?? 0).toBeGreaterThan(zero ?? 0);
    expect(rowTopOf(extents, metrics, 99)).toBeNull();
  });
  it('lands a collapsed section on its header, which is all there is to scroll to', () => {
    const page = fakePage([8, 8]);
    const { extents } = sectionExtents(page, metrics, parseCollapseState(['era:2020']), true);
    expect(rowTopOf(extents, metrics, 3)).toBe(extents[0]?.top);
  });
});
