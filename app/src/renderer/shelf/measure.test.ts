import { describe, expect, it } from 'vitest';
import { mountedIndices } from '../keyboard/gridNavigation.js';
import type { GridState, MountWindow } from '../keyboard/gridNavigation.js';
import { parseCollapseState } from './collapse.js';
import {
  EMPTY_MOUNT_WINDOW,
  GRID_ROW_GAP,
  measureGrid,
  SECTION_HEADER_HEIGHT,
  sectionExtents,
  windowFor,
} from './measure.js';
import type { ShelfPage } from './page.js';

describe('measureGrid', () => {
  it("uses §8.0's formula: W = containerWidth − 44 − 10", () => {
    // 1236 canvas, default tile: six columns, as §8.1 reasons from.
    expect(measureGrid(1236 + 44 + 10, 186).columns).toBe(6);
  });
  it('moves by exactly one column at each density step at the 1,236 px canvas', () => {
    const canvas = 1236 + 44 + 10;
    expect(measureGrid(canvas, 148).columns).toBe(7);
    expect(measureGrid(canvas, 186).columns).toBe(6);
    expect(measureGrid(canvas, 232).columns).toBe(5);
  });
  it('shows why 240 was rejected: it skips a column count', () => {
    const canvas = 1236 + 44 + 10;
    expect(measureGrid(canvas, 240).columns).toBe(4);
  });
  it('never returns fewer than one column', () => {
    expect(measureGrid(20, 232).columns).toBe(1);
    expect(measureGrid(0, 186).columns).toBe(1);
  });
  it('realizes the track by dividing the remaining width, gaps removed', () => {
    const metrics = measureGrid(1236 + 44 + 10, 148);
    expect(metrics.columns).toBe(7);
    expect(Math.round(metrics.trackWidth)).toBe(Math.round((1236 - 16 * 6) / 7));
  });
  it('derives row height from the 2:3 card, so the pitch follows the realized track', () => {
    const metrics = measureGrid(1236 + 44 + 10, 186);
    expect(metrics.rowHeight).toBeCloseTo(metrics.trackWidth * 1.5, 5);
  });
  it('never returns a track a card could not be drawn in', () => {
    expect(measureGrid(0, 186).trackWidth).toBeGreaterThan(0);
    expect(measureGrid(20, 232).rowHeight).toBeGreaterThan(0);
  });
});

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
  return {
    sections,
    reference: [],
    ignored: [],
    matched: counts.reduce((a, b) => a + b, 0),
    renderedTotal: counts.reduce((a, b) => a + b, 0),
    orderKey: 'k',
    generation: 1,
    ast: { grammarVersion: 1, terms: [], ignored: [] },
  } as unknown as ShelfPage;
}

describe('sectionExtents', () => {
  const metrics = { columns: 4, trackWidth: 200, rowHeight: 300 };
  it('sizes the canvas from the counts alone, before a row is mounted', () => {
    const { canvasHeight, extents } = sectionExtents(
      fakePage([8, 4]),
      metrics,
      parseCollapseState([]),
      true,
    );
    expect(extents).toHaveLength(2);
    expect(canvasHeight).toBeGreaterThan(0);
  });
  it('gives a collapsed section its header and no body', () => {
    const { extents } = sectionExtents(
      fakePage([8]),
      metrics,
      parseCollapseState(['era:2020']),
      true,
    );
    expect(extents[0]?.collapsed).toBe(true);
    expect(extents[0]?.bodyHeight).toBe(0);
  });
  it('rounds a part row up: 5 rows in 4 columns is 2 rows of pitch', () => {
    const { extents } = sectionExtents(fakePage([5]), metrics, parseCollapseState([]), true);
    expect(extents[0]?.bodyHeight).toBe(2 * 300 + GRID_ROW_GAP + 18);
  });
  it('stacks the second section below the first', () => {
    const { extents } = sectionExtents(fakePage([4, 4]), metrics, parseCollapseState([]), true);
    expect(extents[1]!.top).toBeGreaterThan(extents[0]!.top + SECTION_HEADER_HEIGHT);
  });
  it('numbers firstIndex over the flat row order, collapsed sections included', () => {
    // A collapsed section still owns its rows; if its count left the running index the flat
    // position of everything below it would shift the moment a chevron was clicked.
    const { extents } = sectionExtents(
      fakePage([8, 4]),
      metrics,
      parseCollapseState(['era:2020']),
      true,
    );
    expect(extents.map((e) => e.firstIndex)).toEqual([0, 8]);
  });
  it('closes the canvas on the last section, so nothing scrolls past the end', () => {
    const { extents, canvasHeight } = sectionExtents(
      fakePage([4, 4]),
      metrics,
      parseCollapseState([]),
      true,
    );
    const last = extents[1]!;
    expect(canvasHeight).toBe(last.top + last.headerHeight + last.bodyHeight);
  });
});

describe('windowFor', () => {
  const metrics = { columns: 4, trackWidth: 200, rowHeight: 300 };
  // 200 rows in an order-10 section auto-collapse past §8.1's 150-item threshold, and a window
  // over a collapsed section is empty — which passes `to < 60` for the wrong reason. The explicit
  // expansion is what keeps every assertion below about windowing.
  const open = parseCollapseState(['!era:2020']);

  it('opens the fixture section, so nothing below passes on an empty window', () => {
    const { extents } = sectionExtents(fakePage([200]), metrics, open, true);
    expect(extents[0]?.collapsed).toBe(false);
    expect(
      sectionExtents(fakePage([200]), metrics, parseCollapseState([]), true).extents[0]?.collapsed,
    ).toBe(true);
  });
  it('mounts only what the viewport plus overscan needs', () => {
    const page = fakePage([200]);
    const { extents } = sectionExtents(page, metrics, open, true);
    const win = windowFor(extents, metrics, 0, 900, page);
    expect(win.from).toBe(0);
    expect(win.to).toBeGreaterThan(0);
    expect(win.to).toBeLessThan(60);
  });
  it('moves the window down as the user scrolls', () => {
    const page = fakePage([200]);
    const { extents } = sectionExtents(page, metrics, open, true);
    const win = windowFor(extents, metrics, 6000, 900, page);
    expect(win.from).toBeGreaterThan(0);
  });
  it('mounts nothing from a collapsed section', () => {
    const page = fakePage([200]);
    const { extents } = sectionExtents(page, metrics, parseCollapseState(['era:2020']), true);
    expect(windowFor(extents, metrics, 0, 900, page)).toEqual(EMPTY_MOUNT_WINDOW);
  });

  // R12: `MountWindow` is 12b's, and `mountedIndices` reads `to` **inclusively**
  // (`gridNavigation.test.ts` pins `{from:1,to:3}` → `[1,2,3]`). These four assertions are the
  // cross-module bar: the window this function returns is fed to that function, and a half-open
  // reading here would mount one extra card per section, silently, forever.
  it('returns a window `mountedIndices` reads back as exactly the rows it named', () => {
    const page = fakePage([200]);
    const { extents } = sectionExtents(page, metrics, open, true);
    const win = windowFor(extents, metrics, 0, 900, page);
    const state: GridState = {
      order: Array.from({ length: 200 }, (_, i) => (i + 1) as unknown as never),
      columns: 4,
    };
    const mounted = mountedIndices(state, win, { projectId: null, desiredColumn: 0 });
    expect(mounted[0]).toBe(win.from);
    expect(mounted.at(-1)).toBe(win.to);
    expect(mounted).toHaveLength(win.to - win.from + 1);
  });
  it('mounts whole grid rows, so the last mounted index closes its row', () => {
    const page = fakePage([200]);
    const { extents } = sectionExtents(page, metrics, open, true);
    const win = windowFor(extents, metrics, 0, 900, page);
    expect(win.to).toBeGreaterThan(0);
    expect((win.to + 1) % metrics.columns).toBe(0);
  });
  it('never names an index the page does not have', () => {
    const page = fakePage([6]);
    const { extents } = sectionExtents(page, metrics, parseCollapseState([]), true);
    const win = windowFor(extents, metrics, 0, 5000, page);
    expect(win.to).toBe(5);
  });
  it('mounts nothing for an empty window, rather than the row at index 0', () => {
    const state: GridState = {
      order: [1 as unknown as never, 2 as unknown as never],
      columns: 4,
    };
    const empty: MountWindow = EMPTY_MOUNT_WINDOW;
    expect(mountedIndices(state, empty, { projectId: null, desiredColumn: 0 })).toEqual([]);
  });
});
