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
import type { ProjectionCapabilities } from './row.js';

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

/**
 * The cycle, in the schema's own variant order.
 *
 * `Completion` is still dropped: nothing computes it and a sort key over an uncomputed column
 * orders by unknown (§8.0a, §8.3a). [p3] §35.2 **generalises that reason rather than overturning
 * it** — `needs_attention` reads a column that is computed for some rows and not others, so the
 * uncomputed rows tail (§35.3) and the key is not offered at all when nothing is computed (§35.5).
 *
 * **This array is a runtime restatement of the enum and cannot be derived**: the generator emits
 * an enum as a bare type union with no runtime value (`protocol/lib/emit-ts.mjs:57`), and only
 * `ERROR_CODES` gets a constant. `viewState.test.ts`'s `AC-P3-35-4` asserts it against
 * `protocol/schema/protocol.json` so the hand-written half is the array and not the claim.
 */
export const SORT_KEYS = ['last_touched', 'name', 'size', 'needs_attention'] as const;

export const SORT_LABELS: Readonly<Record<SortKey, string>> = {
  last_touched: 'Last touched',
  name: 'Name',
  size: 'Size',
  needs_attention: 'Needs attention',
};

/**
 * [p3] §35.5. The keys the control offers, given what the projection can answer.
 *
 * *A control that cannot order anything is the same object as a chip that counts nothing*, and
 * §23.4's rule applies at control granularity: a section that exists only to advertise a feature
 * is an ad. The capability is `projectionCapabilities`' own — *a capability is present when at
 * least one row answers it; an empty projection answers nothing* — and this states no second rule.
 */
export function offeredSorts(caps: ProjectionCapabilities): readonly SortKey[] {
  return SORT_KEYS.filter((key) => key !== 'needs_attention' || caps.health);
}

/**
 * [p3] §35.5. A stored key the projection cannot honour renders as the default.
 *
 * **Nothing is written back.** The resolution happens where the comparator is chosen, so a stored
 * preference is not deleted because today's library cannot honour it, and the key returns the
 * moment a reading exists. `patchFor` issues a `view.set` only when `prev.sort !== next.sort`, so
 * leaving `ShelfView.sort` alone is both the rule and its implementation.
 */
export function resolveSort(stored: SortKey, offered: readonly SortKey[]): SortKey {
  return offered.includes(stored) ? stored : SORT_KEYS[0];
}

/** Cycles over the **offered** set, so a control that does not show a key cannot cycle onto it.
 *  The default is the whole cycle, which is what a caller with no projection means. */
export function nextSort(sort: SortKey, offered: readonly SortKey[] = SORT_KEYS): SortKey {
  const keys = offered.length === 0 ? SORT_KEYS : offered;
  const index = keys.indexOf(sort);
  return keys[(index + 1) % keys.length] ?? SORT_KEYS[0];
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
