import { describe, expect, it } from 'vitest';
import type { ProjectId } from '../../generated/protocol';
import {
  type GridFocus,
  type GridState,
  NO_FOCUS,
  columnIn,
  focusIndex,
  initialFocus,
  mountedIndices,
  moveAcrossSections,
  tabIndexFor,
  type GridDirection,
} from './gridNavigation';

const id = (n: number): ProjectId => n as unknown as ProjectId;

/** One section's grid: every case below predates sections and holds with nothing to cross into. */
const moveFocus = (state: GridState, focus: GridFocus, direction: GridDirection): GridFocus =>
  moveAcrossSections({ sections: [state.order], columns: state.columns }, focus, direction);
const ids = (n: number): ProjectId[] => Array.from({ length: n }, (_, i) => id(i + 1));

// Seven cards over four columns: a full row, a full row, then a short last row of three.
const grid: GridState = { order: ids(7), columns: 4 };
const at = (
  n: number,
  desiredColumn = (n - 1) % 4,
): { projectId: ProjectId; desiredColumn: number } => ({ projectId: id(n), desiredColumn });

describe('focus starts on the first card and is held as an id', () => {
  it('opens on the first card of the sequence', () => {
    expect(initialFocus(grid)).toEqual({ projectId: id(1), desiredColumn: 0 });
    expect(initialFocus({ order: [], columns: 4 })).toEqual(NO_FOCUS);
  });

  it('finds the focused card by id, not by remembered position', () => {
    expect(focusIndex(grid, at(6))).toBe(5);
    // A card inserted at the head renumbers every index; the id still resolves.
    const grown: GridState = { order: [id(99), ...grid.order], columns: 4 };
    expect(focusIndex(grown, at(6))).toBe(6);
  });

  it('reports -1 when the focused id has left the sequence', () => {
    expect(focusIndex(grid, at(42))).toBe(-1);
    expect(focusIndex(grid, NO_FOCUS)).toBe(-1);
  });
});

describe('horizontal movement clamps at row ends and never wraps', () => {
  it('moves within the row', () => {
    expect(moveFocus(grid, at(2), 'right').projectId).toBe(id(3));
    expect(moveFocus(grid, at(3), 'left').projectId).toBe(id(2));
  });

  it('stops at the end of a row instead of falling to the next one', () => {
    expect(moveFocus(grid, at(4), 'right').projectId).toBe(id(4));
    expect(moveFocus(grid, at(5), 'left').projectId).toBe(id(5));
  });

  it('stops at the very ends of the sequence', () => {
    expect(moveFocus(grid, at(1), 'left').projectId).toBe(id(1));
    expect(moveFocus(grid, at(7), 'right').projectId).toBe(id(7));
  });

  it('resets the desired column, because the user has just chosen one', () => {
    expect(moveFocus(grid, at(1, 3), 'right').desiredColumn).toBe(1);
    expect(moveFocus(grid, at(4, 0), 'left').desiredColumn).toBe(2);
  });
});

describe('vertical movement carries a desired column', () => {
  it('moves by the measured column count', () => {
    expect(moveFocus(grid, at(2), 'down').projectId).toBe(id(6));
    expect(moveFocus(grid, at(6), 'up').projectId).toBe(id(2));
  });

  it('clamps into a short last row without changing the desired column', () => {
    // Column 3 of row 1 is card 4; row 2 has only three cards, so focus lands on card 7…
    const landed = moveFocus(grid, at(4), 'down');
    expect(landed.projectId).toBe(id(7));
    expect(landed.desiredColumn).toBe(3);
    // …and coming back up returns to column 3, not to column 2 where the short row left it.
    expect(moveFocus(grid, landed, 'up').projectId).toBe(id(4));
  });

  it('does not drift left across three moves, which is the bug the column exists for', () => {
    let focus: GridFocus = at(4);
    focus = moveFocus(grid, focus, 'down');
    focus = moveFocus(grid, focus, 'up');
    focus = moveFocus(grid, focus, 'down');
    expect(focus.projectId).toBe(id(7));
    expect(focus.desiredColumn).toBe(3);
  });

  it('stops at the top and the bottom row', () => {
    expect(moveFocus(grid, at(2), 'up').projectId).toBe(id(2));
    expect(moveFocus(grid, at(6), 'down').projectId).toBe(id(6));
  });

  it('does nothing at all with no focus or an empty grid', () => {
    expect(moveFocus(grid, NO_FOCUS, 'down')).toEqual(NO_FOCUS);
    expect(moveFocus({ order: [], columns: 4 }, at(1), 'down')).toEqual(NO_FOCUS);
  });
});

// The shape exec-dfui-1 measured the defect on: three columns, A = 1–4, B = 5–7 collapsed, so
// absent from the walk, C = 8–10. Each section starts its own visual row.
describe('vertical movement crosses sections and never enters a collapsed one (§11.7)', () => {
  const shelf = { sections: [[1, 2, 3, 4].map(id), [8, 9, 10].map(id)], columns: 3 };
  const move = (focus: GridFocus, direction: GridDirection): GridFocus =>
    moveAcrossSections(shelf, focus, direction);
  const atCol = (n: number, desiredColumn: number): GridFocus => ({
    projectId: id(n),
    desiredColumn,
  });

  it('goes from a section’s last row to the next section’s first row at the desired column', () => {
    // 4 is A's second row, column 0: down lands on C's first card, not on collapsed 7.
    expect(move(atCol(4, 0), 'down')).toEqual(atCol(8, 0));
    // 3 is A's first row, column 2; A's second row is short, so 3 → 4 clamps and keeps column 2…
    const clamped = move(atCol(3, 2), 'down');
    expect(clamped).toEqual(atCol(4, 2));
    // …and the column survives the crossing into C, instead of landing in collapsed 6.
    expect(move(clamped, 'down')).toEqual(atCol(10, 2));
  });

  it('goes from a section’s first row to the previous section’s last row, clamped', () => {
    // C's first row, column 2 → A's last row holds only card 4.
    const up = move(atCol(10, 2), 'up');
    expect(up).toEqual(atCol(4, 2));
    // The desired column carried through comes back out on the way up again.
    expect(move(up, 'up')).toEqual(atCol(3, 2));
    expect(move(atCol(8, 0), 'up')).toEqual(atCol(4, 0));
  });

  it('keeps left and right inside the section, clamped at row ends', () => {
    expect(move(atCol(4, 0), 'right')).toEqual(atCol(4, 0));
    expect(move(atCol(8, 0), 'left')).toEqual(atCol(8, 0));
    expect(move(atCol(3, 2), 'right')).toEqual(atCol(3, 2));
    expect(move(atCol(9, 1), 'left')).toEqual(atCol(8, 0));
  });

  it('stops at the first and the last open section', () => {
    expect(move(atCol(2, 1), 'up')).toEqual(atCol(2, 1));
    expect(move(atCol(9, 1), 'down')).toEqual(atCol(9, 1));
  });

  it('skips an empty section and has no move for a card outside every open section', () => {
    const gapped = { sections: [[id(1)], [], [id(2)]], columns: 3 };
    expect(moveAcrossSections(gapped, atCol(1, 0), 'down')).toEqual(atCol(2, 0));
    expect(move(atCol(6, 0), 'down')).toEqual(NO_FOCUS);
  });

  it('reads a card’s column from its own section, for a selection no arrow made', () => {
    expect(columnIn(shelf, id(3))).toBe(2);
    expect(columnIn(shelf, id(4))).toBe(0);
    expect(columnIn(shelf, id(10))).toBe(2);
    expect(columnIn(shelf, id(6))).toBe(0);
    expect(columnIn(shelf, null)).toBe(0);
  });

  it('walks a one-column list by row across sections', () => {
    const list = { sections: shelf.sections, columns: 1 };
    expect(moveAcrossSections(list, atCol(4, 0), 'down')).toEqual(atCol(8, 0));
    expect(moveAcrossSections(list, atCol(8, 0), 'up')).toEqual(atCol(4, 0));
    expect(moveAcrossSections(list, atCol(8, 0), 'right')).toEqual(atCol(8, 0));
    expect(moveAcrossSections(list, atCol(8, 0), 'left')).toEqual(atCol(8, 0));
  });
});

describe('one cell carries tabindex 0 and the rest -1', () => {
  it('so Tab re-enters where it left', () => {
    expect(tabIndexFor(id(3), at(3))).toBe(0);
    expect(tabIndexFor(id(4), at(3))).toBe(-1);
    expect(tabIndexFor(id(1), NO_FOCUS)).toBe(-1);
  });
});

describe('the virtualizer may not evict the focused card', () => {
  it('mounts the window', () => {
    expect(mountedIndices(grid, { from: 1, to: 3 }, at(2))).toEqual([1, 2, 3]);
  });

  it('adds the focused card when it has scrolled out, above or below', () => {
    expect(mountedIndices(grid, { from: 4, to: 6 }, at(1))).toEqual([0, 4, 5, 6]);
    expect(mountedIndices(grid, { from: 0, to: 2 }, at(7))).toEqual([0, 1, 2, 6]);
  });

  it('adds nothing when the focused id is not in the sequence', () => {
    expect(mountedIndices(grid, { from: 0, to: 2 }, at(42))).toEqual([0, 1, 2]);
    expect(mountedIndices(grid, { from: 0, to: 2 }, NO_FOCUS)).toEqual([0, 1, 2]);
  });

  it('clamps the window to the sequence', () => {
    expect(mountedIndices(grid, { from: -3, to: 40 }, NO_FOCUS)).toEqual([0, 1, 2, 3, 4, 5, 6]);
  });
});
