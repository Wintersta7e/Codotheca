import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId, ProjectRow } from '../../generated/protocol.js';
import { mountedIndices, type MountWindow } from '../keyboard/gridNavigation.js';
import type { QueryContext } from '../shelf/evaluate.js';
import { sectionExtents } from '../shelf/measure.js';
import { buildShelfPage, type ShelfPage } from '../shelf/page.js';
import { projectionCapabilities, toShelfRow, type ShelfRow } from '../shelf/row.js';
import { useVirtualizer } from '../shelf/useVirtualizer.js';
import type { VirtualizerInput, VirtualizerState } from '../shelf/useVirtualizer.js';
import { DEFAULT_SHELF_VIEW } from '../shelf/viewState.js';
import { makeProjectRow } from '../testing/projectRow.js';
import { ShelfScreen, type ShelfScreenProps } from './ShelfScreen.js';
import { fakeAppDeps } from './testDeps.js';

vi.mock('../card/ProjectCard.js', () => ({
  ProjectCard: ({ row, now }: { row: ShelfRow; now: number }) => (
    <div data-testid="shelf-card" data-id={row.id} data-now={now}>
      {row.name}
    </div>
  ),
}));

vi.mock('../shelf/useVirtualizer.js', () => ({
  useVirtualizer: vi.fn(),
}));

const NOW = Date.UTC(2026, 6, 1, 12) / 1000;
const DAY = 86_400;
const METRICS = { columns: 3, trackWidth: 200, rowHeight: 300 };
const NO_COLLECTIONS: ReadonlyMap<string, number> = new Map();
let mountWindow: MountWindow;

function row(overrides: Partial<ProjectRow> = {}): ShelfRow {
  return {
    ...toShelfRow(makeProjectRow(overrides)),
    authoredByUser: overrides.isReference === true ? false : true,
  };
}

function rowsInEra(count: number, firstId = 1): readonly ShelfRow[] {
  return Array.from({ length: count }, (_, index) =>
    row({
      id: (firstId + index) as ProjectId,
      name: `project-${String(firstId + index)}`,
      lastTouchedAt: NOW - DAY,
    }),
  );
}

function context(rows: readonly ShelfRow[], now = NOW): QueryContext {
  return {
    now,
    firstRunCompletedAt: null,
    collectionIdsByName: NO_COLLECTIONS,
    pathsAreCaseSensitive: false,
    capabilities: projectionCapabilities(rows),
    commitSubjectHits: null,
  };
}

function pageFor(rows: readonly ShelfRow[], view = DEFAULT_SHELF_VIEW, now = NOW): ShelfPage {
  return buildShelfPage({
    rows,
    query: view.query,
    sort: view.sort,
    now,
    generation: 7,
    ctx: context(rows, now),
  });
}

function propsFor(
  rows: readonly ShelfRow[],
  overrides: Partial<ShelfScreenProps> = {},
): ShelfScreenProps {
  const fake = fakeAppDeps({}, { effectsTier: 'off' });
  fake.setNow(NOW);
  return {
    deps: fake.deps,
    rows,
    generation: 7,
    view: DEFAULT_SHELF_VIEW,
    onViewChange: vi.fn(),
    notices: [],
    scan: null,
    sessions: new Map(),
    firstRunCompletedAt: null,
    tier: 'off',
    onOpenProject: vi.fn(),
    onOpenPalette: vi.fn(),
    onOpenSettings: vi.fn(),
    onOpenScanSummary: vi.fn(),
    onAddScanRoot: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  mountWindow = { from: 0, to: 10_000 };
  vi.mocked(useVirtualizer).mockImplementation((input: VirtualizerInput): VirtualizerState => {
    const sized = sectionExtents(input.page, METRICS, input.collapse, input.queryIsEmpty);
    return { ...sized, metrics: METRICS, mountWindow };
  });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe('ShelfScreen', () => {
  it('renders era headers in the page section order', () => {
    const rows = [
      row({ id: 1 as ProjectId, name: 'live', lastTouchedAt: NOW - DAY }),
      row({ id: 2 as ProjectId, name: 'month', lastTouchedAt: NOW - 14 * DAY }),
      row({ id: 3 as ProjectId, name: 'quarter', lastTouchedAt: NOW - 45 * DAY }),
      row({
        id: 4 as ProjectId,
        name: 'earlier',
        lastTouchedAt: Date.UTC(2026, 0, 15) / 1000,
      }),
      row({
        id: 5 as ProjectId,
        name: 'prior-year',
        lastTouchedAt: Date.UTC(2025, 5, 1) / 1000,
      }),
      row({ id: 6 as ProjectId, name: 'archived', isArchived: true }),
    ];
    const expected = pageFor(rows).sections.map((section) => section.label);

    const { container } = render(<ShelfScreen {...propsFor(rows)} />);
    const rendered = [...container.querySelectorAll('.cdt-era-label')].map(
      (element) => element.textContent,
    );

    expect(expected.length).toBeGreaterThan(3);
    expect(rendered).toEqual(expected);
  });

  it('renders every reference row below the grid without a cap', () => {
    const reference = Array.from({ length: 43 }, (_, index) =>
      row({
        id: (100 + index) as ProjectId,
        name: `reference-${String(index + 1)}`,
        isReference: true,
      }),
    );
    const rows = [...rowsInEra(5), ...reference];

    const { container } = render(<ShelfScreen {...propsFor(rows)} />);
    const tail = container.querySelector('.cdt-reference-tail');
    const body = container.querySelector('.cdt-shelf-body');
    const grids = [...container.querySelectorAll('.cdt-shelf-grid')];

    expect(container.querySelectorAll('.cdt-reference-row')).toHaveLength(reference.length);
    expect(grids.at(-1)?.contains(tail)).toBe(false);
    expect(body?.lastElementChild).toBe(tail);
  });

  it('mounts exactly the inclusive virtualizer window', () => {
    const rows = rowsInEra(8);
    const page = pageFor(rows);
    const flatRows = page.sections.flatMap((section) => section.rows);
    mountWindow = { from: 1, to: 3 };
    const focus = { projectId: null, desiredColumn: 0 };
    const state = { order: flatRows.map((entry) => entry.id), columns: METRICS.columns };
    const expectedIds = mountedIndices(state, mountWindow, focus).map(
      (index) => flatRows[index]?.id,
    );

    const { getAllByTestId } = render(<ShelfScreen {...propsFor(rows)} />);
    const renderedIds = getAllByTestId('shelf-card').map((card) =>
      Number(card.getAttribute('data-id')),
    );

    expect(expectedIds).toEqual([2, 3, 4]);
    expect(renderedIds).toEqual(expectedIds);
  });

  it('swaps the grid for one list over the same flat row set', () => {
    const rows = rowsInEra(7);
    const expectedNames = pageFor(rows).sections.flatMap((section) =>
      section.rows.map((entry) => entry.name),
    );
    const props = propsFor(rows);
    const { container, rerender } = render(<ShelfScreen {...props} />);
    const gridNames = [...container.querySelectorAll('[data-testid="shelf-card"]')].map(
      (element) => element.textContent,
    );

    expect(gridNames).toEqual(expectedNames);
    expect(container.querySelector('.cdt-list')).toBeNull();

    rerender(<ShelfScreen {...props} view={{ ...props.view, viewMode: 'list' }} />);
    const listNames = [...container.querySelectorAll('.cdt-list-row .cdt-list-c3')].map(
      (element) => element.textContent,
    );

    expect(container.querySelector('.cdt-shelf-grid')).toBeNull();
    expect(container.querySelectorAll('.cdt-list')).toHaveLength(1);
    expect(listNames).toEqual(expectedNames);
  });

  it('re-reads the injected clock every second and clears the interval', () => {
    vi.useFakeTimers();
    const fake = fakeAppDeps({}, { effectsTier: 'off' });
    fake.setNow(NOW);
    const props = propsFor(rowsInEra(4), { deps: fake.deps });
    const { getAllByTestId, unmount } = render(<ShelfScreen {...props} />);

    expect(getAllByTestId('shelf-card')[0]?.getAttribute('data-now')).toBe(String(NOW));
    act(() => {
      fake.setNow(NOW + 1);
      vi.advanceTimersByTime(1000);
    });
    expect(getAllByTestId('shelf-card')[0]?.getAttribute('data-now')).toBe(String(NOW + 1));

    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
});
