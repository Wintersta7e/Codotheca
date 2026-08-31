import type { MountWindow } from '../keyboard/gridNavigation.js';
import type { CollapseState } from './collapse.js';
import { isCollapsed } from './collapse.js';
import type { ShelfPage } from './page.js';

/** §7.7's Provenance clause: two tokens, not a scale. Density never scales them. */
export const GRID_COL_GAP = 16;
export const GRID_ROW_GAP = 24;
/** §8.0: side padding 22 + 22, plus the 10px scrollbar. */
export const SHELF_SIDE_PADDING = 44;
export const SCROLLBAR_WIDTH = 10;
export const SECTION_HEADER_HEIGHT = 52;
export const SECTION_MARGIN_TOP = 26;
export const GRID_PADDING_TOP = 18;
export const OVERSCAN_ROWS = 2;
/** The card is 2:3, so the row pitch follows the realized track (§7.7). */
export const CARD_ASPECT = 1.5;

export interface GridMetrics {
  readonly columns: number;
  readonly trackWidth: number;
  readonly rowHeight: number;
}

export function measureGrid(containerWidth: number, tile: number): GridMetrics {
  const usable = containerWidth - SHELF_SIDE_PADDING - SCROLLBAR_WIDTH;
  const columns = Math.max(1, Math.floor((usable + GRID_COL_GAP) / (tile + GRID_COL_GAP)));
  const trackWidth = Math.max(1, (usable - GRID_COL_GAP * (columns - 1)) / columns);
  return { columns, trackWidth, rowHeight: trackWidth * CARD_ASPECT };
}

export interface SectionExtent {
  readonly id: string;
  readonly top: number;
  readonly headerHeight: number;
  readonly bodyHeight: number;
  readonly rowCount: number;
  readonly collapsed: boolean;
  /** Index of this section's first row in the page's flat row order. */
  readonly firstIndex: number;
}

export function sectionExtents(
  page: ShelfPage,
  metrics: GridMetrics,
  collapse: CollapseState,
  queryIsEmpty: boolean,
): { readonly extents: readonly SectionExtent[]; readonly canvasHeight: number } {
  const extents: SectionExtent[] = [];
  let top = 0;
  let firstIndex = 0;
  for (const section of page.sections) {
    const collapsed = isCollapsed(
      collapse,
      section.id,
      section.order,
      page.renderedTotal,
      queryIsEmpty,
    );
    const gridRows = Math.ceil(section.agg.count / metrics.columns);
    const bodyHeight = collapsed
      ? 0
      : GRID_PADDING_TOP + gridRows * metrics.rowHeight + Math.max(0, gridRows - 1) * GRID_ROW_GAP;
    top += SECTION_MARGIN_TOP;
    extents.push({
      id: section.id,
      top,
      headerHeight: SECTION_HEADER_HEIGHT,
      bodyHeight,
      rowCount: section.agg.count,
      collapsed,
      firstIndex,
    });
    top += SECTION_HEADER_HEIGHT + bodyHeight;
    // A collapsed section keeps its slice of the flat order: dropping it would move every row
    // below it the moment a chevron was clicked, and focus is resolved through that order.
    firstIndex += section.agg.count;
  }
  return { extents, canvasHeight: top };
}

// R12: `MountWindow` is 12b's, imported at the top of this file. A second identical interface
// here would type-check and still be two owners of one shape — 12b's `mountedIndices` takes the
// window this function returns, so they must be the same declaration.
//
// **`to` is inclusive**, because `mountedIndices` reads it that way and its tests pin it
// (`gridNavigation.test.ts`: `{from:1,to:3}` → `[1,2,3]`). A half-open reading here type-checks
// and mounts one extra card per section forever, which is why the empty window is spelled
// `to < from` rather than `{from:0,to:0}` — that pair names index 0, not nothing.
export const EMPTY_MOUNT_WINDOW: MountWindow = { from: 0, to: -1 };

/**
 * The window is over the page's **flat** row order, so a focused project id maps to one index
 * regardless of which section holds it (§11.7).
 */
export function windowFor(
  extents: readonly SectionExtent[],
  metrics: GridMetrics,
  scrollTop: number,
  viewportHeight: number,
  page: ShelfPage,
): MountWindow {
  const viewTop = scrollTop;
  const viewBottom = scrollTop + viewportHeight;
  let from = Number.POSITIVE_INFINITY;
  let to = -1;

  for (const extent of extents) {
    if (extent.collapsed || extent.rowCount === 0) continue;
    const bodyTop = extent.top + extent.headerHeight + GRID_PADDING_TOP;
    const bodyBottom = extent.top + extent.headerHeight + extent.bodyHeight;
    if (bodyBottom < viewTop || bodyTop > viewBottom) continue;

    const pitch = metrics.rowHeight + GRID_ROW_GAP;
    const firstRow = Math.max(0, Math.floor((viewTop - bodyTop) / pitch) - OVERSCAN_ROWS);
    const lastRow = Math.floor((viewBottom - bodyTop) / pitch) + OVERSCAN_ROWS;
    const start = extent.firstIndex + firstRow * metrics.columns;
    const end = Math.min(
      extent.firstIndex + extent.rowCount - 1,
      extent.firstIndex + (lastRow + 1) * metrics.columns - 1,
    );
    from = Math.min(from, Math.max(extent.firstIndex, start));
    to = Math.max(to, end);
  }

  if (!Number.isFinite(from) || to < from) return EMPTY_MOUNT_WINDOW;
  const total = page.sections.reduce((sum, section) => sum + section.agg.count, 0);
  return { from, to: Math.min(to, total - 1) };
}
