import type { ProjectId } from '../../generated/protocol';

/**
 * §11.7: **the grid is a grid.** The prototype moves the four arrows over one flat array, which
 * makes `↑`/`↓` behave as `←`/`→` and is wrong on a wrapped layout.
 *
 * `order` is the visible sequence and already excludes collapsed sections. `columns` is the
 * count the virtualizer measured to size its scroll canvas (§8.2) — this module reads no DOM
 * and derives no column width.
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

function focusAt(state: GridState, index: number, desiredColumn: number): GridFocus {
  const projectId = state.order[index];
  if (projectId === undefined) return NO_FOCUS;
  return { projectId, desiredColumn };
}

export function moveFocus(state: GridState, focus: GridFocus, direction: GridDirection): GridFocus {
  const columns = Math.max(1, Math.trunc(state.columns));
  const index = focusIndex(state, focus);
  if (index < 0) return NO_FOCUS;

  const column = index % columns;
  const row = Math.trunc(index / columns);
  const lastIndex = state.order.length - 1;

  if (direction === 'left' || direction === 'right') {
    // Clamp at row ends without wrapping, and reset the desired column: the user just chose one.
    const target = direction === 'left' ? column - 1 : column + 1;
    if (target < 0 || target >= columns) return { ...focus, desiredColumn: column };
    const moved = row * columns + target;
    if (moved > lastIndex) return { ...focus, desiredColumn: column };
    return focusAt(state, moved, target);
  }

  const desired = Math.min(columns - 1, Math.max(0, focus.desiredColumn));
  const targetRow = direction === 'up' ? row - 1 : row + 1;
  if (targetRow < 0) return { ...focus, desiredColumn: desired };

  const rowStart = targetRow * columns;
  if (rowStart > lastIndex) return { ...focus, desiredColumn: desired };
  // A short last row clamps the landing without touching the desired column, so coming back up
  // returns to the column the user was in rather than the one the short row could offer.
  const landing = Math.min(rowStart + desired, lastIndex);
  return focusAt(state, landing, desired);
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
