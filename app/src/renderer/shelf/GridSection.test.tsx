import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId, ProjectRow } from '../../generated/protocol.js';
import { CARD_ROLE } from '../a11y/names.js';
import { GridSection, gridRunsOf } from './GridSection.js';
import type { GridSectionProps } from './GridSection.js';
import type { SectionExtent } from './measure.js';
import type { ShelfSection } from './page.js';
import { toShelfRow } from './row.js';

vi.mock('../card/ProjectCard.js', () => ({
  // The mock mirrors the real card's own `tabIndex={focused ? 0 : -1}` and its `gridcell` role
  // (`ProjectCard.tsx:99-100`). A mock that dropped them would make the roving-tabindex and
  // role assertions below pass over a shape the product does not have.
  ProjectCard: ({
    row,
    focused,
    selected,
    now,
    haloOpacity,
  }: {
    row: { id: number; name: string };
    focused: boolean;
    selected: boolean;
    now: number;
    haloOpacity: number;
  }) => (
    <div
      data-testid="card"
      role="gridcell"
      tabIndex={focused ? 0 : -1}
      data-id={row.id}
      data-focused={focused}
      data-selected={selected}
      data-now={now}
      data-halo={haloOpacity}
    >
      {row.name}
    </div>
  ),
}));

// RTL's auto-cleanup only registers under `globals: true`, which this repo does not set, so a
// document-wide query would otherwise read the previous test's cards (`ProjectCard.test.tsx:16`).
afterEach(cleanup);

const metrics = { columns: 4, trackWidth: 200, rowHeight: 300 };
function section(count: number): ShelfSection {
  return {
    id: 'era:2020',
    order: 16,
    year: 2020,
    cutAgainstYear: 2026,
    label: '2020',
    agg: {
      count,
      trackedBytes: 0,
      indexedCount: count,
      unpushed: 0,
      uncommitted: 0,
      interrupted: 0,
      unchecked: 0,
    },
    rows: Array.from({ length: count }, (_, i) => ({ id: i + 1, name: `p${String(i + 1)}` })),
  } as unknown as ShelfSection;
}
const extent: SectionExtent = {
  id: 'era:2020',
  top: 26,
  headerHeight: 52,
  bodyHeight: 4000,
  rowCount: 60,
  collapsed: false,
  firstIndex: 0,
};

// `MountWindow.to` is INCLUSIVE — 12b's `mountedIndices` reads it that way and its own tests pin
// it (`gridNavigation.test.ts:110`). `{from:0,to:7}` is eight cards, not nine.
const props = (over: Partial<GridSectionProps> = {}): GridSectionProps => ({
  section: section(60),
  extent,
  metrics,
  mountWindow: { from: 0, to: 7 },
  focus: { projectId: null, desiredColumn: 0 },
  density: 186,
  now: 0,
  firstRunCompletedAt: null,
  selectedId: null,
  sessions: new Map(),
  halo: null,
  peek: null,
  onActivate: () => {},
  onOpen: () => {},
  onTogglePin: () => {},
  onStopSession: () => {},
  ...over,
});
const draw = (over: Partial<GridSectionProps> = {}): ReturnType<typeof render> =>
  render(<GridSection {...props(over)} />);

describe('gridRunsOf', () => {
  it('groups a contiguous window into grid rows', () => {
    expect(gridRunsOf([0, 1, 2, 3, 4], 4)).toEqual([
      { rowIndex: 0, indices: [0, 1, 2, 3] },
      { rowIndex: 1, indices: [4] },
    ]);
  });
  it('keeps a distant focused cell as its own run rather than mounting the gap', () => {
    expect(gridRunsOf([0, 1, 40], 4)).toEqual([
      { rowIndex: 0, indices: [0, 1] },
      { rowIndex: 10, indices: [40] },
    ]);
  });
  it('is empty for an empty window, and mounts no row wrapper for one', () => {
    expect(gridRunsOf([], 4)).toEqual([]);
  });
});

describe('GridSection', () => {
  it('mounts only the window, not the section', () => {
    draw();
    expect(screen.getAllByTestId('card')).toHaveLength(8);
  });
  it('is a grid of rows, so a virtualized position can be announced', () => {
    draw();
    const grid = screen.getByRole('grid');
    expect(grid.getAttribute('aria-rowcount')).toBe('15');
    expect(grid.getAttribute('aria-colcount')).toBe('4');
    expect(screen.getAllByRole('row')[0]?.getAttribute('aria-rowindex')).toBe('1');
  });
  it('never evicts the focused project, however far it is scrolled out (§11.7)', () => {
    draw({ focus: { projectId: 41 as unknown as ProjectId, desiredColumn: 0 } });
    const ids = screen.getAllByTestId('card').map((el) => el.getAttribute('data-id'));
    expect(ids).toContain('41');
    // …and it is announced at its real grid row, not appended to the window's last row.
    const rows = screen.getAllByRole('row');
    expect(rows.at(-1)?.getAttribute('aria-rowindex')).toBe('11');
  });
  it('carries the offset as height, so the scroll canvas is the counts and not the DOM', () => {
    // Rows 5 and 6 of fifteen, at a 300 + 24 pitch. The leading spacer must be the grid's FIRST
    // child and carry five rows of it — querying `.cdt-shelf-spacer` loosely finds the trailing
    // one instead, and then a leading spacer of zero passes.
    const { container } = draw({ mountWindow: { from: 20, to: 27 } });
    const pitch = metrics.rowHeight + 24;
    const children = [...container.querySelectorAll('.cdt-shelf-grid > *')];
    const leading = children[0] as HTMLElement | undefined;
    expect(leading?.className).toBe('cdt-shelf-spacer');
    expect(Number.parseFloat(leading?.style.height ?? '0')).toBe(5 * pitch);
    const trailing = children.at(-1) as HTMLElement | undefined;
    expect(trailing?.className).toBe('cdt-shelf-spacer');
    expect(Number.parseFloat(trailing?.style.height ?? '0')).toBe((15 - 7) * pitch);
  });
  it('draws no leading spacer when the window starts at the top', () => {
    const { container } = draw();
    expect((container.querySelector('.cdt-shelf-grid > *') as HTMLElement).className).toBe(
      'cdt-shelf-grid-row',
    );
  });
  it('draws no body at all when the section is collapsed', () => {
    const { container } = draw({ extent: { ...extent, collapsed: true, bodyHeight: 0 } });
    expect(container.querySelector('.cdt-shelf-grid')).toBeNull();
    expect(screen.queryAllByTestId('card')).toHaveLength(0);
  });
  it("puts Peek below the selected card's row, spanning the grid", () => {
    const { container } = draw({ selectedId: 2 as unknown as ProjectId, peek: <p>peek</p> });
    const slot = container.querySelector('.cdt-shelf-peek-slot');
    expect(slot?.textContent).toBe('peek');
    const rows = [...container.querySelectorAll('.cdt-shelf-grid > *')];
    expect(rows.indexOf(slot as Element)).toBeGreaterThan(0);
  });
  it('opens Peek under the row that holds the selection, not the first row', () => {
    const { container } = draw({ selectedId: 6 as unknown as ProjectId, peek: <p>peek</p> });
    const children = [...container.querySelectorAll('.cdt-shelf-grid > *')];
    const slot = container.querySelector('.cdt-shelf-peek-slot') as Element;
    const owningRow = children[children.indexOf(slot) - 1];
    expect(owningRow?.getAttribute('aria-rowindex')).toBe('2');
  });
  it('draws no Peek slot at all when nothing is selected', () => {
    const { container } = draw({ peek: <p>peek</p> });
    expect(container.querySelector('.cdt-shelf-peek-slot')).toBeNull();
  });
  it('marks exactly one cell tabbable', () => {
    const { container } = draw({
      focus: { projectId: 3 as unknown as ProjectId, desiredColumn: 2 },
    });
    expect([...container.querySelectorAll('[tabindex="0"]')]).toHaveLength(1);
  });
  it('hands the shelf’s instant down unchanged, and a later one advances it', () => {
    // The card cannot invent a later second: a shelf that passes a constant freezes every bench
    // row and every as-of clause on the grid (12c's gap G3).
    const { rerender } = draw({ now: 1_770_000_000 });
    expect(screen.getAllByTestId('card')[0]?.getAttribute('data-now')).toBe('1770000000');
    rerender(<GridSection {...props({ now: 1_770_000_600 })} />);
    expect(screen.getAllByTestId('card')[0]?.getAttribute('data-now')).toBe('1770000600');
  });
  it('dips exactly the one card the shelf chose, and leaves the rest at full', () => {
    // §11.6 dips at most one card in the viewport, chosen above the grid. Passing a constant here
    // makes the whole clamp inert and no stylesheet says so.
    draw({ halo: { projectId: 2 as unknown as ProjectId, opacity: 0.42 } });
    const halos = screen
      .getAllByTestId('card')
      .map((el) => [el.getAttribute('data-id'), el.getAttribute('data-halo')]);
    expect(halos).toContainEqual(['2', '0.42']);
    expect(halos.filter(([, h]) => h !== '1')).toHaveLength(1);
  });
  it('leaves every card at full opacity when no card is dipping', () => {
    draw({ halo: null });
    const halos = screen.getAllByTestId('card').map((el) => el.getAttribute('data-halo'));
    expect(new Set(halos)).toEqual(new Set(['1']));
  });
  it('mounts to the end of a section when the window runs past it', () => {
    draw({
      section: section(6),
      extent: { ...extent, rowCount: 6 },
      mountWindow: { from: 0, to: 99 },
    });
    expect(screen.getAllByTestId('card')).toHaveLength(6);
  });
  it('sets the density on the namespaced property the stylesheet may legally read', () => {
    // `check-style-tokens.mjs:118-122` rejects a `var(--x)` that is neither in `tokens.css` nor
    // prefixed `cdt-`, so `--tile` is a property the grid could set and no rule could ever read.
    const { container } = draw({ density: 232 });
    const grid = container.querySelector('.cdt-shelf-grid') as HTMLElement;
    expect(grid.style.getPropertyValue('--cdt-tile')).toBe('232px');
    expect(grid.style.getPropertyValue('--tile')).toBe('');
  });
  it('reports the Peek slot’s measured height, so the canvas can grow by it', () => {
    const onPeekHeight = vi.fn();
    draw({ selectedId: 2 as unknown as ProjectId, peek: <p>peek</p>, onPeekHeight });
    expect(onPeekHeight).toHaveBeenCalled();
  });
});

/**
 * The mock above cannot prove the roving tabindex reaches a real card, because the mock is what
 * sets it. This block mounts the real component once — the shape the product actually has.
 */
describe('GridSection over the real card', () => {
  const realRow = (id: number): ProjectRow =>
    toShelfRow({
      id,
      name: `p${String(id)}`,
      owner: null,
      description: null,
      descriptionSource: null,
      birthYear: null,
      primaryLanguage: null,
      archetype: null,
      artSceneHash: null,
      artState: 'pending',
      conditionSignal: null,
      completionLit: null,
      completionApplicable: null,
      isPinned: false,
      isArchived: false,
      isHidden: false,
      isReference: false,
      isFork: false,
      isBare: false,
      isShallow: false,
      isSubmodule: false,
      ambiguousLineage: false,
      lastTouchedAt: 0,
      lastInteractionAt: null,
      lastCommitAt: null,
      lastCommitSubject: null,
      firstCommitAt: null,
      createdAt: 0,
      acknowledgedAt: null,
      sizeTrackedBytes: null,
      trackedFiles: null,
      collectionIds: [],
      primaryLocation: null,
      presence: 'present',
      branch: null,
      isDirty: null,
      untrackedCount: null,
      ahead: null,
      behind: null,
      stashCount: null,
      interruptedOp: null,
      fetchHeadAt: null,
      refstateObservedAt: null,
      worktreeObservedAt: null,
      errorKind: null,
      errorAt: null,
      eraSectionId: 'era:2020',
    } as unknown as ProjectRow);

  it('mounts real cards as gridcells, exactly one of them tabbable', async () => {
    vi.doUnmock('../card/ProjectCard.js');
    vi.resetModules();
    const { GridSection: Real } = await import('./GridSection.js');
    const rows = [realRow(1), realRow(2), realRow(3), realRow(4)];
    const { container } = render(
      <Real
        {...props({
          section: { ...section(4), rows } as unknown as ShelfSection,
          extent: { ...extent, rowCount: 4 },
          mountWindow: { from: 0, to: 3 },
          focus: { projectId: 3 as unknown as ProjectId, desiredColumn: 2 },
        })}
      />,
    );
    // Discriminate against the mock first, or this whole test passes over the mock: the mock also
    // renders a `gridcell` with a roving tabindex, and `.cdt-card` is the real component's.
    expect(container.querySelectorAll('.cdt-card')).toHaveLength(4);
    expect(container.querySelector('[data-testid="card"]')).toBeNull();

    expect(container.querySelectorAll(`[role="${CARD_ROLE}"]`)).toHaveLength(4);
    expect(container.querySelectorAll('[tabindex="0"]')).toHaveLength(1);
    // The card renders no completion readout at all in phase 1 — an absent rank, never a zero.
    expect(container.textContent).not.toMatch(/0\s*\/\s*10/);
  });
});
