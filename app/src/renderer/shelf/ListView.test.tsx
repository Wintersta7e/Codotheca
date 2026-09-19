import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { LIST_COLUMNS, ListView } from './ListView.js';
import type { ListViewProps } from './ListView.js';
import { listRowChips } from './chips.js';
import type { ShelfRow } from './row.js';

afterEach(cleanup);

const NOW = 1_770_000_000;
const row = (over: Record<string, unknown> = {}): ShelfRow =>
  ({
    id: 1,
    name: 'atlas',
    primaryLanguage: 'Rust',
    branch: 'main',
    sizeTrackedBytes: null,
    conditionSignal: null,
    isArchived: false,
    isReference: false,
    interruptedOp: null,
    refstateObservedAt: NOW,
    isDirty: null,
    worktreeObservedAt: null,
    ahead: null,
    behind: null,
    fetchHeadAt: null,
    createdAt: 0,
    acknowledgedAt: 0,
    // [p3] §31.7: the two columns the score and the rank mark read. Both NULL together, which is
    // the uncomputed state every project starts in — the wire pairs them and §1.10's CHECK does.
    completionLit: null,
    completionApplicable: null,
    primaryLocation: { id: 10, pathDisplay: '/w/atlas' },
    ...over,
  }) as unknown as ShelfRow;

const draw = (over: Partial<ListViewProps> = {}): ReturnType<typeof render> =>
  render(
    <ListView
      rows={[row()]}
      now={NOW}
      firstRunCompletedAt={0}
      selectedId={null}
      peek={null}
      onActivate={() => {}}
      onOpen={() => {}}
      {...over}
    />,
  );

describe('ListView', () => {
  // [p3] §31.7 fills both reserved columns. **The widths are unchanged**, which is what makes
  // this a fill and not a re-cut — and an uncomputed row still renders no node in either, which
  // is the half that survives from phase 1.
  it('holds columns 2 and 7 at their reserved widths and renders no node while uncomputed', () => {
    const { container } = draw();
    for (const [cls, width] of [
      ['.cdt-list-c2', 16],
      ['.cdt-list-c7', 44],
    ] as const) {
      // Scoped to the row: the header now carries the same class, because both columns gained a
      // label in the same change that gave them a value.
      const cell = container.querySelector<HTMLElement>(`.cdt-list-row > ${cls}`);
      if (cell === null) throw new Error(`missing ${cls}`);
      expect(cell.childNodes).toHaveLength(0);
      expect(cell.textContent).toBe('');
      expect(
        LIST_COLUMNS.find((column) => `.cdt-list-c${String(column.index)}` === cls)?.widthPx,
      ).toBe(width);
    }
  });

  it('fills both columns once a measurement exists', () => {
    const { container } = draw({
      rows: [row({ completionLit: 8, completionApplicable: 8 })],
    });
    expect(container.querySelector('.cdt-list-row > .cdt-list-c7')?.textContent).toBe('8/8');
    expect(container.querySelector('.cdt-list-row .cdt-list-rank')).not.toBeNull();
    // Never a percentage, and never a numerator without its denominator.
    expect(container.querySelector('.cdt-list-row > .cdt-list-c7')?.textContent).not.toContain('%');
  });

  // [p3] §31.7: both reserved columns gain a header cell — an unlabelled column carrying a
  // value is a value with no name.
  it('labels columns 2 to 7', () => {
    const { container } = draw();
    const headers = [...container.querySelectorAll('.cdt-list-header > *')]
      .map((element) => element.textContent)
      .filter(Boolean);
    expect(headers).toEqual(['RANK', 'NAME', 'LANG', 'BRANCH', 'SIZE', 'SCORE']);
  });

  it('dashes the size column where inventory has not run, and only there', () => {
    const { container } = draw();
    expect(container.querySelector('.cdt-list-row .cdt-list-c6')?.textContent).toBe('—');
    // §31.7: an uncomputed score renders NOTHING, never a dash — a dash in a column that could
    // have been empty is a claim with no measurement behind it.
    expect(container.querySelector('.cdt-list-row > .cdt-list-c2')?.textContent).toBe('');
    expect(container.querySelector('.cdt-list-row > .cdt-list-c7')?.textContent).toBe('');
  });

  it('draws no condition dot when condition_signal is NULL', () => {
    const { container } = draw();
    expect(container.querySelector('.cdt-list-row > .cdt-list-c1')?.childNodes).toHaveLength(0);
  });

  it('draws the dot from §5.4a alone when there is a signal', () => {
    const { container } = draw({ rows: [row({ conditionSignal: 'dormant' })] });
    expect(container.querySelector('.cdt-condition-dot')?.getAttribute('aria-label')).toContain(
      'dormant',
    );
  });

  // §8.6: roasting stays inside an opened project card, and the list draws none.
  // [p3] §31.7 fills the score column, and it is a bare fraction: the `EVALUABLE` and `UNKNOWN`
  // words are band 5's, on a surface served by `projects.get`, and the list is not one.
  it('never draws a roast, and never band 5 words, on the list', () => {
    const { container } = draw({
      rows: [row({ completionLit: 8, completionApplicable: 8 })],
    });
    expect(container.querySelector('.cdt-roast')).toBeNull();
    expect(container.textContent).not.toMatch(/EVALUABLE|UNKNOWN/);
  });

  it('opens Peek below the selected row as a direct list child', () => {
    const selected = row().id;
    const { container } = draw({ selectedId: selected, peek: <p>peek</p> });
    const directChildren = [...container.querySelectorAll('.cdt-list > *')];
    const slot = container.querySelector('.cdt-shelf-peek-slot');
    if (slot === null) throw new Error('missing Peek slot');
    expect(directChildren.indexOf(slot)).toBeGreaterThan(1);
  });

  it('gives the project row an accessible name', () => {
    draw();
    expect(screen.getByRole('row', { name: /atlas/ })).toBeTruthy();
  });

  it('prints a measured zero but not an unmeasured size', () => {
    const measured = draw({ rows: [row({ sizeTrackedBytes: 0 })] }).container;
    const unmeasured = draw({ rows: [row({ sizeTrackedBytes: null })] }).container;
    const measuredText = measured.querySelector('.cdt-list-row .cdt-list-c6')?.textContent;
    const unmeasuredText = unmeasured.querySelector('.cdt-list-row .cdt-list-c6')?.textContent;
    expect(measuredText).toBe('0 MB');
    expect(unmeasuredText).toBe('—');
    expect(measuredText).not.toBe(unmeasuredText);
  });

  it('activates on double click and opens on single click', () => {
    const onActivate = vi.fn();
    const onOpen = vi.fn();
    const id = row().id;
    draw({ onActivate, onOpen });
    const projectRow = screen.getByRole('row', { name: /atlas/ });

    fireEvent.click(projectRow);
    expect(onOpen).toHaveBeenCalledWith(id);
    expect(onActivate).not.toHaveBeenCalled();

    fireEvent.doubleClick(projectRow);
    expect(onActivate).toHaveBeenCalledWith(id);
    expect(onOpen).toHaveBeenCalledOnce();
  });

  it('creates header cells only for columns that have labels', () => {
    const { container } = draw();
    const header = container.querySelector('.cdt-list-header');
    if (header === null) throw new Error('missing list header');
    expect(header.childElementCount).toBe(
      LIST_COLUMNS.filter((column) => column.header !== null).length,
    );
    // [p3] §31.7: columns 2 and 7 now have labels, so they now have header cells. Columns 1 and
    // 8 still do not — the condition dot and the chip strip name themselves.
    expect(header.querySelector('.cdt-list-c2')).not.toBeNull();
    expect(header.querySelector('.cdt-list-c7')).not.toBeNull();
    expect(header.querySelector('.cdt-list-c1')).toBeNull();
    expect(header.querySelector('.cdt-list-c8')).toBeNull();
  });

  it("renders exactly listRowChips' set in the chip column", () => {
    const flagged = row({
      interruptedOp: 'merge',
      isDirty: true,
      worktreeObservedAt: NOW,
    });
    const { container } = draw({ rows: [flagged] });
    const rendered = [...container.querySelectorAll('.cdt-list-c8 [data-chip]')].map((chip) =>
      chip.getAttribute('data-chip'),
    );
    expect(rendered).toEqual(listRowChips(flagged, NOW, 0).map((chip) => chip.id));
  });
});
