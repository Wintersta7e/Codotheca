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
  it('holds columns 2 and 7 at their widths and renders no node in either', () => {
    const { container } = draw();
    for (const [cls, width] of [
      ['.cdt-list-c2', 16],
      ['.cdt-list-c7', 44],
    ] as const) {
      const cell = container.querySelector<HTMLElement>(cls);
      if (cell === null) throw new Error(`missing ${cls}`);
      expect(cell.childNodes).toHaveLength(0);
      expect(cell.textContent).toBe('');
      expect(
        LIST_COLUMNS.find((column) => `.cdt-list-c${String(column.index)}` === cls)?.widthPx,
      ).toBe(width);
    }
  });

  it('labels columns 3 to 6 only', () => {
    const { container } = draw();
    const headers = [...container.querySelectorAll('.cdt-list-header > *')]
      .map((element) => element.textContent)
      .filter(Boolean);
    expect(headers).toEqual(['NAME', 'LANG', 'BRANCH', 'SIZE']);
  });

  it('dashes the size column where inventory has not run, and only there', () => {
    const { container } = draw();
    expect(container.querySelector('.cdt-list-row .cdt-list-c6')?.textContent).toBe('—');
    expect(container.querySelector('.cdt-list-c2')?.textContent).toBe('');
  });

  it('draws no condition dot when condition_signal is NULL', () => {
    const { container } = draw();
    expect(container.querySelector('.cdt-list-c1')?.childNodes).toHaveLength(0);
  });

  it('draws the dot from §5.4a alone when there is a signal', () => {
    const { container } = draw({ rows: [row({ conditionSignal: 'dormant' })] });
    expect(container.querySelector('.cdt-condition-dot')?.getAttribute('aria-label')).toContain(
      'dormant',
    );
  });

  it('never draws a roast or a completion readout on the list', () => {
    const { container } = draw();
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
    expect(header.querySelector('.cdt-list-c2')).toBeNull();
    expect(header.querySelector('.cdt-list-c7')).toBeNull();
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
