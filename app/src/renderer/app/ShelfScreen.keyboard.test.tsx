import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { type ReactElement, useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId } from '../../generated/protocol.js';
import { required } from '../../shared/required.js';
import type { QueryContext } from '../shelf/evaluate.js';
import { firstVisibleIndex, sectionExtents } from '../shelf/measure.js';
import { buildShelfPage } from '../shelf/page.js';
import { projectionCapabilities, toShelfRow, type ShelfRow } from '../shelf/row.js';
import { SHELF_SCROLL_CLASS } from '../shelf/Shelf.js';
import {
  rowTopOf,
  useVirtualizer,
  type VirtualizerInput,
  type VirtualizerState,
} from '../shelf/useVirtualizer.js';
import { DEFAULT_SHELF_VIEW, type ShelfView } from '../shelf/viewState.js';
import { withProjectDeps } from '../testing/deps.js';
import { makeProjectRow } from '../testing/projectRow.js';
import { ShelfScreen, type ShelfScreenProps } from './ShelfScreen.js';
import { fakeAppDeps } from './testDeps.js';

// §11.7 through the real card: the roving `tabindex`, the focus handler and the window's key
// listener are the shapes under test, so only the virtualizer's DOM measurement is replaced —
// jsdom lays nothing out — and it answers from the same pure geometry the real hook uses.
vi.mock('../shelf/useVirtualizer.js', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../shelf/useVirtualizer.js')>()),
  useVirtualizer: vi.fn(),
}));

const NOW = Date.UTC(2026, 6, 1, 12) / 1000;
const DAY = 86_400;
const METRICS = { columns: 3, trackWidth: 200, rowHeight: 300 };
const VIEWPORT = 400;
let scrollTop = 0;

const ROWS: readonly ShelfRow[] = Array.from({ length: 9 }, (_, index) => ({
  ...toShelfRow(
    makeProjectRow({
      id: (index + 1) as ProjectId,
      name: `project-${String(index + 1)}`,
      lastTouchedAt: NOW - DAY,
    }),
  ),
  authoredByUser: true,
}));

const CONTEXT: QueryContext = {
  now: NOW,
  firstRunCompletedAt: null,
  collectionIdsByName: new Map(),
  pathsAreCaseSensitive: false,
  capabilities: projectionCapabilities(ROWS),
  commitSubjectHits: null,
};

/** The scroll offset of the grid row holding flat index `index`, from the shelf's own geometry. */
function rowTop(index: number): number {
  const page = buildShelfPage({
    rows: ROWS,
    query: '',
    sort: DEFAULT_SHELF_VIEW.sort,
    now: NOW,
    generation: 7,
    ctx: CONTEXT,
  });
  const { extents } = sectionExtents(page, METRICS, DEFAULT_SHELF_VIEW.collapsed, true);
  return required(rowTopOf(extents, METRICS, index), 'a laid-out row');
}

function Harness(props: { readonly initial: ShelfView }): ReactElement {
  const [view, setView] = useState(props.initial);
  const fake = fakeAppDeps({}, { effectsTier: 'off' });
  fake.setNow(NOW);
  const screen: ShelfScreenProps = {
    deps: fake.deps,
    rows: ROWS,
    library: 'present',
    problems: null,
    generation: 7,
    view,
    onViewChange: setView,
    notices: [],
    scan: null,
    sessions: new Map(),
    firstRunCompletedAt: null,
    tier: 'off',
    onOpenProject: vi.fn(),
    motivation: null,
    route: 'shelf',
    onOpenProfile: vi.fn(),
    onOpenPalette: vi.fn(),
    onOpenSettings: vi.fn(),
    onOpenScanSummary: vi.fn(),
    onAddScanRoot: vi.fn(),
  };
  return <ShelfScreen {...screen} />;
}

function mount(initial: ShelfView = DEFAULT_SHELF_VIEW): HTMLElement {
  const { container } = render(<Harness initial={initial} />, { wrapper: withProjectDeps() });
  return container;
}

function cards(container: HTMLElement): readonly HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>('.cdt-card')];
}

function card(container: HTMLElement, name: string): HTMLElement {
  return required(
    cards(container).find((element) => element.textContent.includes(name)),
    `the card for ${name}`,
  );
}

function rovingCells(container: HTMLElement): readonly HTMLElement[] {
  return cards(container).filter((element) => element.getAttribute('tabindex') === '0');
}

beforeEach(() => {
  scrollTop = 0;
  vi.mocked(useVirtualizer).mockImplementation((input: VirtualizerInput): VirtualizerState => {
    const sized = sectionExtents(input.page, METRICS, input.collapse, input.queryIsEmpty);
    return {
      ...sized,
      metrics: METRICS,
      mountWindow: { from: 0, to: 10_000 },
      firstVisible: firstVisibleIndex(sized.extents, METRICS, scrollTop),
    };
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('starting a selection on a fresh shelf (§11.7)', () => {
  it('mounts nine cards, so a passing run is never one over an empty grid', () => {
    expect(cards(mount())).toHaveLength(9);
  });

  it('gives the first visible card the one tabindex="0", so Tab can enter the grid', () => {
    // Scrolled so the first row is wholly above the viewport: the entry is row 1, not index 0.
    scrollTop = rowTop(3);
    const container = mount();
    expect(rovingCells(container)).toEqual([card(container, 'project-4')]);
  });

  it('selects the first visible card on the first arrow, and moves focus onto it', () => {
    scrollTop = rowTop(3);
    const container = mount();
    // A fresh shelf: focus is on the document body, a real target with a real tag (R45).
    expect(document.activeElement).toBe(document.body);
    fireEvent.keyDown(document.body, { key: 'ArrowDown', code: 'ArrowDown' });

    const entry = card(container, 'project-4');
    expect(entry.getAttribute('data-focused')).toBe('true');
    expect(document.activeElement).toBe(entry);
    expect(rovingCells(container)).toEqual([entry]);

    // The next arrow moves from there, and focus follows the selection.
    fireEvent.keyDown(entry, { key: 'ArrowRight', code: 'ArrowRight' });
    const moved = card(container, 'project-5');
    expect(moved.getAttribute('data-focused')).toBe('true');
    expect(entry.getAttribute('data-focused')).toBe('false');
    expect(document.activeElement).toBe(moved);
  });

  it('makes a card reached by Tab the selection, and Space then opens Peek', () => {
    const container = mount();
    const [entry] = rovingCells(container);
    // jsdom has no Tab traversal; focusing the one tabindex="0" cell is what Tab does.
    act(() => {
      required(entry, 'the roving cell').focus();
    });
    expect(required(entry, 'the roving cell').getAttribute('data-focused')).toBe('true');
    expect(container.querySelector('.cdt-shelf-peek-slot')).toBeNull();

    fireEvent.keyDown(required(entry, 'the roving cell'), { key: ' ', code: 'Space' });
    expect(container.querySelector('.cdt-shelf-peek-slot')).not.toBeNull();
  });

  it('scrolls an off-screen selection into view when Space opens its Peek', () => {
    // A selection restored with the view, two rows below a 400px viewport at the top.
    const container = mount({ ...DEFAULT_SHELF_VIEW, selectedProjectId: 9 as ProjectId });
    const scroller = required(
      container.querySelector<HTMLElement>(`.${SHELF_SCROLL_CLASS}`),
      'the shelf scroller',
    );
    Object.defineProperty(scroller, 'clientHeight', { configurable: true, value: VIEWPORT });
    Object.defineProperty(scroller, 'scrollTop', { configurable: true, writable: true, value: 0 });

    fireEvent.keyDown(document.body, { key: ' ', code: 'Space' });

    const top = rowTop(8);
    expect(top + METRICS.rowHeight).toBeGreaterThan(VIEWPORT);
    expect(scroller.scrollTop).toBe(top + METRICS.rowHeight - VIEWPORT);
    expect(container.querySelector('.cdt-shelf-peek-slot')).not.toBeNull();
  });
});
