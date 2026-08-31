import { type RefObject, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ProjectId } from '../../generated/protocol.js';
import type { MountWindow } from '../keyboard/gridNavigation.js';
import type { CollapseState } from './collapse.js';
import type { GridMetrics, SectionExtent } from './measure.js';
import {
  EMPTY_MOUNT_WINDOW,
  GRID_PADDING_TOP,
  GRID_ROW_GAP,
  measureGrid,
  sectionExtents,
  windowFor,
} from './measure.js';
import type { ShelfPage } from './page.js';

export function sameWindow(a: MountWindow, b: MountWindow): boolean {
  return a.from === b.from && a.to === b.to;
}

/**
 * Peek opens inline below the selected card, so the section it opened in is taller than
 * `sectionExtents` sized it from counts alone. Without this the canvas is short by Peek's height
 * and the last row cannot be scrolled to.
 */
export function applyPeekHeight(
  extents: readonly SectionExtent[],
  canvasHeight: number,
  sectionId: string,
  height: number,
): { readonly extents: readonly SectionExtent[]; readonly canvasHeight: number } {
  if (height <= 0) return { extents, canvasHeight };
  let shift = 0;
  const grown = extents.map((extent) => {
    const top = extent.top + shift;
    if (extent.id !== sectionId) return top === extent.top ? extent : { ...extent, top };
    shift += height;
    return { ...extent, top, bodyHeight: extent.bodyHeight + height };
  });
  return { extents: grown, canvasHeight: canvasHeight + shift };
}

/** The scroll offset of the grid row holding a flat row index, or null if it is not laid out. */
export function rowTopOf(
  extents: readonly SectionExtent[],
  metrics: GridMetrics,
  flatIndex: number,
): number | null {
  for (const extent of extents) {
    const local = flatIndex - extent.firstIndex;
    if (local < 0 || local >= extent.rowCount) continue;
    if (extent.collapsed) return extent.top;
    const gridRow = Math.floor(local / metrics.columns);
    return (
      extent.top +
      extent.headerHeight +
      GRID_PADDING_TOP +
      gridRow * (metrics.rowHeight + GRID_ROW_GAP)
    );
  }
  return null;
}

export interface VirtualizerInput {
  readonly page: ShelfPage;
  readonly tile: number;
  readonly collapse: CollapseState;
  readonly queryIsEmpty: boolean;
  readonly focusedProjectId: ProjectId | null;
  readonly peekExtra: { readonly sectionId: string; readonly height: number } | null;
  readonly scrollRef: RefObject<HTMLElement | null>;
}

export interface VirtualizerState {
  readonly metrics: GridMetrics;
  readonly extents: readonly SectionExtent[];
  readonly canvasHeight: number;
  readonly mountWindow: MountWindow;
}

export function useVirtualizer(input: VirtualizerInput): VirtualizerState {
  const { page, tile, collapse, queryIsEmpty, focusedProjectId, peekExtra, scrollRef } = input;
  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  const [mountWindow, setMountWindow] = useState<MountWindow>(EMPTY_MOUNT_WINDOW);
  const frameRef = useRef<number | null>(null);

  const metrics = measureGrid(viewport.width, tile);
  const sized = sectionExtents(page, metrics, collapse, queryIsEmpty);
  const { extents, canvasHeight } = peekExtra
    ? applyPeekHeight(sized.extents, sized.canvasHeight, peekExtra.sectionId, peekExtra.height)
    : sized;

  // The commit point. A scroll that leaves the window where it was must not set state — that is
  // the whole of the DOM budget probe C measured at 1,000 cards.
  const commit = useCallback((next: MountWindow) => {
    setMountWindow((prev) => (sameWindow(prev, next) ? prev : next));
  }, []);

  const recompute = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    commit(windowFor(extents, metrics, el.scrollTop, el.clientHeight, page));
  }, [commit, extents, metrics, page, scrollRef]);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return undefined;
    const measure = (): void => {
      setViewport((prev) =>
        prev.width === el.clientWidth && prev.height === el.clientHeight
          ? prev
          : { width: el.clientWidth, height: el.clientHeight },
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => {
      observer.disconnect();
    };
  }, [scrollRef]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return undefined;
    // Coalesced to one frame: a wheel gesture fires far more scroll events than frames, and a
    // handler that recomputes per event is the shelf's only realistic way to miss one.
    const onScroll = (): void => {
      if (frameRef.current !== null) return;
      frameRef.current = requestAnimationFrame(() => {
        frameRef.current = null;
        recompute();
      });
    };
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      el.removeEventListener('scroll', onScroll);
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    };
  }, [recompute, scrollRef]);

  useLayoutEffect(() => {
    recompute();
  }, [recompute]);

  // §8.0a: a density change re-anchors on the focused project id, never the pixel offset — the
  // offset addresses a different row once the grid re-cuts.
  const previousTile = useRef(tile);
  useLayoutEffect(() => {
    if (previousTile.current === tile) return;
    previousTile.current = tile;
    const el = scrollRef.current;
    if (!el || focusedProjectId === null) return;
    let flat = -1;
    let seen = 0;
    for (const section of page.sections) {
      const local = section.rows.findIndex((row) => row.id === focusedProjectId);
      if (local >= 0) {
        flat = seen + local;
        break;
      }
      seen += section.agg.count;
    }
    const top = flat < 0 ? null : rowTopOf(extents, metrics, flat);
    if (top !== null) el.scrollTop = Math.max(0, top - el.clientHeight / 2);
  }, [extents, focusedProjectId, metrics, page, scrollRef, tile]);

  return { metrics, extents, canvasHeight, mountWindow };
}
