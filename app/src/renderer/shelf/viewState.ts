import type {
  ProjectId,
  SortKey,
  ViewMode,
  ViewPatch,
  ViewState,
} from '../../generated/protocol.js';
import type { QueryAst } from '../../shared/query/ast.js';
import { parseQuery } from '../../shared/query/parse.js';
import type { CollapseState } from './collapse.js';
import { parseCollapseState, serializeCollapseState } from './collapse.js';

/** The shelf's own reading of `view_state`: the query parsed once, the collapse strings turned
 *  into state, and the density snapped onto the ladder. */
export interface ShelfView {
  readonly query: string;
  readonly ast: QueryAst;
  readonly sort: SortKey;
  readonly viewMode: ViewMode;
  readonly density: number;
  readonly collapsed: CollapseState;
  readonly scrollOffset: number;
  readonly selectedProjectId: ProjectId | null;
  readonly dismissedNotices: readonly string[];
}

/** §8.0a's three steps, one per §7.7 rendering band (`<156`, `156–205`, `≥206`). There is no
 *  range and no free value: a fourth value inside a band changes no column count and names
 *  nothing a reader can say. `densityStep` maps one of these back to its band. */
export const DENSITY_TILE_PX = [148, 186, 232] as const;
export const DEFAULT_DENSITY_PX = 186;

export function clampDensity(px: number | null | undefined): number {
  if (typeof px !== 'number' || !Number.isFinite(px) || px <= 0) return DEFAULT_DENSITY_PX;
  let best: number = DEFAULT_DENSITY_PX;
  for (const step of DENSITY_TILE_PX) {
    if (Math.abs(step - px) < Math.abs(best - px)) best = step;
  }
  return best;
}

export function nextDensity(px: number): number {
  const current = clampDensity(px);
  const index = DENSITY_TILE_PX.indexOf(current as (typeof DENSITY_TILE_PX)[number]);
  return DENSITY_TILE_PX[(index + 1) % DENSITY_TILE_PX.length] ?? DEFAULT_DENSITY_PX;
}

/** §8.0a drops `Completion`: nothing computes it in phase 1 and a sort key over an uncomputed
 *  column orders by unknown. §8.3a made the same ruling for the query field. */
export const SORT_KEYS = ['last_touched', 'name', 'size'] as const;

export const SORT_LABELS: Readonly<Record<SortKey, string>> = {
  last_touched: 'Last touched',
  name: 'Name',
  size: 'Size',
};

export function nextSort(sort: SortKey): SortKey {
  const index = SORT_KEYS.indexOf(sort);
  return SORT_KEYS[(index + 1) % SORT_KEYS.length] ?? 'last_touched';
}

export function viewFromState(state: ViewState): ShelfView {
  return {
    query: state.query,
    ast: parseQuery(state.query),
    sort: state.sort,
    viewMode: state.viewMode,
    density: clampDensity(state.density),
    collapsed: parseCollapseState(state.collapsedSections),
    scrollOffset: state.scrollOffset,
    selectedProjectId: state.selectedProjectId,
    dismissedNotices: state.dismissedNotices,
  };
}

export const DEFAULT_SHELF_VIEW: ShelfView = {
  query: '',
  ast: parseQuery(''),
  sort: 'last_touched',
  viewMode: 'grid',
  density: DEFAULT_DENSITY_PX,
  collapsed: new Map(),
  scrollOffset: 0,
  selectedProjectId: null,
  dismissedNotices: [],
};

function sameStrings(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

/** Returns the fields that changed and `null` for the rest, per `ViewPatch`'s stated meaning.
 *  Returns `null` outright when nothing changed, so an idle shelf issues no `view.set`.
 *
 *  `windowGeometry` is always `null`: §11.2 gives it to the shell, and the renderer has no
 *  reading of it to write back. */
export function patchFor(prev: ShelfView, next: ShelfView): ViewPatch | null {
  const collapsedNext = serializeCollapseState(next.collapsed);
  const collapsedPrev = serializeCollapseState(prev.collapsed);
  const densityChanged = prev.density !== next.density;
  const patch: ViewPatch = {
    query: prev.query === next.query ? null : next.query,
    sort: prev.sort === next.sort ? null : next.sort,
    viewMode: prev.viewMode === next.viewMode ? null : next.viewMode,
    density: densityChanged ? next.density : null,
    collapsedSections: sameStrings(collapsedPrev, collapsedNext) ? null : collapsedNext,
    // §8.0a: a density change re-anchors on the focused project id, never on the pixel offset —
    // once the grid re-cuts, the same offset addresses a different row.
    scrollOffset:
      densityChanged || prev.scrollOffset === next.scrollOffset ? null : next.scrollOffset,
    selectedProjectId:
      prev.selectedProjectId === next.selectedProjectId ? null : next.selectedProjectId,
    dismissedNotices: sameStrings(prev.dismissedNotices, next.dismissedNotices)
      ? null
      : next.dismissedNotices,
    windowGeometry: null,
  };
  const changed = Object.values(patch).some((value) => value !== null);
  return changed ? patch : null;
}
