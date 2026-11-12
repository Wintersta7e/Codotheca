import type { ProjectId } from '../../generated/protocol';

/**
 * §11.7: **the grid is a grid.** The prototype moves the four arrows over one flat array, which
 * makes `↑`/`↓` behave as `←`/`→` and is wrong on a wrapped layout.
 *
 * `columns` is the count the virtualizer measured to size its scroll canvas (§8.2) — this module
 * reads no DOM and derives no column width.
 */
export interface GridFocus {
  /** Focus is a project **id**, never a row index: an insert mid-scan renumbers every index. */
  readonly projectId: ProjectId | null;
  /** Carried across vertical moves so a short last row does not drift focus left. */
  readonly desiredColumn: number;
}

export type GridDirection = 'up' | 'down' | 'left' | 'right';

export interface GridState {
  readonly order: readonly ProjectId[];
  readonly columns: number;
}

/**
 * The shelf as §11.7 walks it: one id list per section that is **not collapsed**, in page order.
 * Each section starts a new visual row, so a column is counted from its own section's first card
 * and never from the shelf's — a collapsed section is simply absent, never stepped into.
 */
export interface SectionedGrid {
  readonly sections: readonly (readonly ProjectId[])[];
  readonly columns: number;
}

export const NO_FOCUS: GridFocus = { projectId: null, desiredColumn: 0 };

export function focusIndex(state: GridState, focus: GridFocus): number {
  if (focus.projectId === null) return -1;
  return state.order.indexOf(focus.projectId);
}

export function initialFocus(state: GridState): GridFocus {
  const first = state.order[0];
  if (first === undefined) return NO_FOCUS;
  return { projectId: first, desiredColumn: 0 };
}

/** The column a card sits in within its own section's rows; 0 for no card or one outside every
 *  open section. */
export function columnIn(grid: SectionedGrid, projectId: ProjectId | null): number {
  if (projectId === null) return 0;
  const columns = Math.max(1, Math.trunc(grid.columns));
  const section = grid.sections.find((ids) => ids.includes(projectId));
  return section === undefined ? 0 : section.indexOf(projectId) % columns;
}

/**
 * `←`/`→` stay inside the focused card's section and clamp at its row ends. `↓` from a section's
 * last row lands on the next section's first row, `↑` from its first row on the previous
 * section's last row — each at the desired column, clamped to that row's length, and the desired
 * column itself survives the crossing.
 */
export function moveAcrossSections(
  grid: SectionedGrid,
  focus: GridFocus,
  direction: GridDirection,
): GridFocus {
  const columns = Math.max(1, Math.trunc(grid.columns));
  const projectId = focus.projectId;
  if (projectId === null) return NO_FOCUS;
  const sections = grid.sections.filter((ids) => ids.length > 0);
  const at = sections.findIndex((ids) => ids.includes(projectId));
  const section = sections[at];
  if (section === undefined) return NO_FOCUS;

  const index = section.indexOf(projectId);
  const column = index % columns;
  const row = Math.trunc(index / columns);
  const lastIndex = section.length - 1;

  if (direction === 'left' || direction === 'right') {
    // Clamp at row ends without wrapping, and reset the desired column: the user just chose one.
    const target = direction === 'left' ? column - 1 : column + 1;
    if (target < 0 || target >= columns) return { ...focus, desiredColumn: column };
    const moved = row * columns + target;
    if (moved > lastIndex) return { ...focus, desiredColumn: column };
    const next = section[moved];
    return next === undefined ? NO_FOCUS : { projectId: next, desiredColumn: target };
  }

  const desired = Math.min(columns - 1, Math.max(0, focus.desiredColumn));
  const lastRowOf = (ids: readonly ProjectId[]): number => Math.trunc((ids.length - 1) / columns);
  const targetRow = direction === 'up' ? row - 1 : row + 1;
  let into: readonly ProjectId[] | undefined = section;
  let landingRow = targetRow;
  if (targetRow < 0) {
    into = sections[at - 1];
    landingRow = into === undefined ? 0 : lastRowOf(into);
  } else if (targetRow > lastRowOf(section)) {
    into = sections[at + 1];
    landingRow = 0;
  }
  if (into === undefined) return { ...focus, desiredColumn: desired };
  // A short row clamps the landing without touching the desired column, so coming back returns
  // to the column the user was in rather than the one the short row could offer.
  const next = into[Math.min(landingRow * columns + desired, into.length - 1)];
  return next === undefined ? NO_FOCUS : { projectId: next, desiredColumn: desired };
}

export function tabIndexFor(projectId: ProjectId, focus: GridFocus): 0 | -1 {
  return focus.projectId === projectId ? 0 : -1;
}

export interface MountWindow {
  readonly from: number;
  readonly to: number;
}

/**
 * The mounted set is `window.from → window.to` **plus the focused project**. Scrolled out of the
 * window the browser drops focus to `<body>` and the keyboard dies silently, so the virtualizer
 * may not evict it.
 */
export function mountedIndices(
  state: GridState,
  window: MountWindow,
  focus: GridFocus,
): readonly number[] {
  const lastIndex = state.order.length - 1;
  const from = Math.max(0, window.from);
  const to = Math.min(lastIndex, window.to);
  const mounted: number[] = [];
  for (let i = from; i <= to; i += 1) mounted.push(i);

  const focused = focusIndex(state, focus);
  if (focused >= 0 && (focused < from || focused > to)) {
    mounted.push(focused);
    mounted.sort((a, b) => a - b);
  }
  return mounted;
}
