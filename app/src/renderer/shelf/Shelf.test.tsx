import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Problems, ProjectId, ScanRunId } from '../../generated/protocol.js';
import css from './shelf.css?raw';
import type { ShelfProps } from './Shelf.js';
import { SHELF_SCROLL_CLASS, Shelf } from './Shelf.js';
import { DEFAULT_SHELF_VIEW } from './viewState.js';

afterEach(cleanup);

/** §11.1's report as the summary panel receives it. `null` is a count nothing computed. */
const problemsWith = (count: number | null): Problems => ({
  runId: 4 as ScanRunId,
  header: {
    walkedDirs: 10,
    repositories: 2,
    problemCount: count,
    ambiguousLineageCount: null,
  },
  groups: [],
});

const emptyPage = {
  sections: [],
  reference: [],
  ignored: [],
  matched: 0,
  renderedTotal: 0,
  orderKey: '',
  generation: 1,
  ast: DEFAULT_SHELF_VIEW.ast,
};

const counts = {
  matched: 0,
  total: 180,
  reference: 24,
  classified: 141,
  classificationKnown: true,
};

const props = (over: Partial<ShelfProps> = {}): ShelfProps => ({
  view: DEFAULT_SHELF_VIEW,
  page: emptyPage,
  counts,
  notices: [],
  scan: { running: false, foundRepos: 0, problemCount: 0 },
  problems: null,
  library: 'present',
  now: 1_760_000_000,
  onViewChange: vi.fn(),
  onOpenPalette: vi.fn(),
  onOpenSettings: vi.fn(),
  onScan: vi.fn(),
  onOpenScanSummary: vi.fn(),
  onAddScanRoot: vi.fn(),
  children: <div data-testid="body" />,
  ...over,
});

describe('the four blocks', () => {
  it('keeps the top bar outside the scroll container, so it never scrolls away', () => {
    const { container } = render(<Shelf {...props()} />);
    const scroll = container.querySelector(`.${SHELF_SCROLL_CLASS}`);
    expect(container.querySelector('.cdt-shelf-bar')).toBeTruthy();
    expect(scroll?.contains(container.querySelector('.cdt-shelf-bar'))).toBe(false);
  });

  it('gives blocks 2–4 exactly one scroll container', () => {
    const { container } = render(<Shelf {...props()} />);
    expect(container.querySelectorAll(`.${SHELF_SCROLL_CLASS}`)).toHaveLength(1);
  });

  it('renders no wrapper for the notice slot when nothing qualifies', () => {
    const { container } = render(<Shelf {...props()} />);
    expect(container.querySelector('.cdt-shelf-notice-slot')).toBeNull();
  });

  it('puts the notice inside the scroll container, above the body', () => {
    const notices = [
      { kind: 'problems' as const, scope: 'run-7', title: 'PROBLEMS', body: 'b', actions: [] },
    ];
    const { container } = render(<Shelf {...props({ notices })} />);
    const scroll = container.querySelector(`.${SHELF_SCROLL_CLASS}`) as HTMLElement;
    expect(scroll.firstElementChild?.className).toBe('cdt-shelf-notice-slot');
  });

  it('fills the vacated space with nothing', () => {
    // No placeholder band, no dimmed rings, no "arriving in a later release" strip.
    const { container } = render(<Shelf {...props()} />);
    const scroll = container.querySelector(`.${SHELF_SCROLL_CLASS}`) as HTMLElement;
    expect(scroll.firstElementChild?.getAttribute('data-testid')).toBe('body');
    expect(container.textContent).not.toMatch(/LVL|RHYTHM|TONIGHT|AMNESTY/i);
  });

  it('publishes the density as the grid track floor on the scroll container', () => {
    const { container } = render(<Shelf {...props()} />);
    const scroll = container.querySelector(`.${SHELF_SCROLL_CLASS}`) as HTMLElement;
    expect(scroll.style.getPropertyValue('--cdt-tile')).toBe('186px');
  });

  it('publishes the step the view holds, not a fixed default', () => {
    const view = { ...DEFAULT_SHELF_VIEW, density: 232 };
    const { container } = render(<Shelf {...props({ view })} />);
    const scroll = container.querySelector(`.${SHELF_SCROLL_CLASS}`) as HTMLElement;
    expect(scroll.style.getPropertyValue('--cdt-tile')).toBe('232px');
  });

  it('replaces the body with the empty state, inside block 4', () => {
    const { container, queryByTestId } = render(<Shelf {...props({ library: 'empty' })} />);
    expect(queryByTestId('body')).toBeNull();
    const empty = container.querySelector('.cdt-shelf-empty');
    expect(container.querySelector(`.${SHELF_SCROLL_CLASS}`)?.contains(empty)).toBe(true);
  });

  it('shows the empty state for a query that matched nothing, and only then', () => {
    const filtered = { ...DEFAULT_SHELF_VIEW, query: 'is:dirty' };
    const { container, queryByTestId } = render(<Shelf {...props({ view: filtered })} />);
    expect(queryByTestId('body')).toBeNull();
    expect(container.querySelector('.cdt-shelf-empty')).toBeTruthy();
  });

  it('renders the body when rows matched, whatever the query is', () => {
    const filtered = { ...DEFAULT_SHELF_VIEW, query: 'is:dirty' };
    const page = { ...emptyPage, matched: 3 };
    const { queryByTestId } = render(<Shelf {...props({ view: filtered, page })} />);
    expect(queryByTestId('body')).toBeTruthy();
  });

  it('offers the scan summary only when the report it opens has problems in it', () => {
    const { container } = render(
      <Shelf {...props({ library: 'empty', problems: problemsWith(3) })} />,
    );
    expect(container.querySelectorAll('.cdt-shelf-empty-action')).toHaveLength(2);
  });

  it('offers no scan summary when the problem count was never computed', () => {
    // `null` is *not computed*, and it is not zero. An uncomputed run is not evidence of a
    // clean one, so it earns no link.
    const { container } = render(
      <Shelf {...props({ library: 'empty', problems: problemsWith(null) })} />,
    );
    expect(container.querySelectorAll('.cdt-shelf-empty-action')).toHaveLength(1);
  });

  it('offers no scan summary when the report has not been read, whatever the scan counted', () => {
    // The shipped dead control: the link was offered from `scan.status` and the panel it opens
    // is drawn from `problems.list`. A run that counted three problems whose report never
    // arrived drew a button that silently did nothing (§11.3a).
    const scan = { running: false, foundRepos: 0, problemCount: 3 };
    const { container } = render(<Shelf {...props({ library: 'empty', scan, problems: null })} />);
    expect(container.querySelectorAll('.cdt-shelf-empty-action')).toHaveLength(1);
  });

  it('names no destructive operation anywhere in the tree', () => {
    const { container } = render(<Shelf {...props()} />);
    expect(container.innerHTML).not.toMatch(/FORGET/i);
  });
});

describe('the keyboard seam', () => {
  it('opens the palette on the one binding that crosses contexts', () => {
    const onOpenPalette = vi.fn();
    render(<Shelf {...props({ onOpenPalette })} />);
    const event = new KeyboardEvent('keydown', {
      key: ' ',
      code: 'Space',
      altKey: true,
      bubbles: true,
      cancelable: true,
    });
    window.dispatchEvent(event);
    expect(onOpenPalette).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(true);
  });

  it('lets a declined key through, so the query field still receives it', () => {
    const { container } = render(<Shelf {...props()} />);
    const input = container.querySelector('.cdt-shelf-field-input') as HTMLElement;
    input.focus();
    const event = new KeyboardEvent('keydown', {
      key: 'p',
      code: 'KeyP',
      bubbles: true,
      cancelable: true,
    });
    input.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it('does not swallow a key no one is listening for', () => {
    // Claiming an action and preventing its default while nothing acts on it is a dead key.
    const { container } = render(<Shelf {...props()} />);
    const event = new KeyboardEvent('keydown', {
      key: 'ArrowDown',
      code: 'ArrowDown',
      bubbles: true,
      cancelable: true,
    });
    container.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it('claims and forwards a grid action once a handler exists', () => {
    const onKeyAction = vi.fn();
    const { container } = render(
      <Shelf
        {...props({
          view: { ...DEFAULT_SHELF_VIEW, selectedProjectId: 7 as ProjectId },
          onKeyAction,
        })}
      />,
    );
    const event = new KeyboardEvent('keydown', {
      key: 'ArrowDown',
      code: 'ArrowDown',
      bubbles: true,
      cancelable: true,
    });
    container.dispatchEvent(event);
    expect(onKeyAction).toHaveBeenCalledWith('shelf.moveDown');
    expect(event.defaultPrevented).toBe(true);
  });

  it('removes its listener when it unmounts', () => {
    const onOpenPalette = vi.fn();
    const { unmount } = render(<Shelf {...props({ onOpenPalette })} />);
    unmount();
    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: ' ', code: 'Space', altKey: true, bubbles: true }),
    );
    expect(onOpenPalette).not.toHaveBeenCalled();
  });
});

describe('shelf.css', () => {
  it('is really read, so nothing below it passes on an empty string', () => {
    expect(css.length).toBeGreaterThan(1000);
    expect(css).toContain('.cdt-shelf-bar');
  });

  it('declares no colour of its own', () => {
    // Criterion 46: every colour resolves to a token declared in §8.7's block.
    const literals = css.match(/#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\(|oklch\(/g) ?? [];
    expect(literals).toEqual([]);
  });

  it('declares no duration and no curve the spec does not carry', () => {
    // Compared as numbers. A CSS formatter rewrites `.16s` as `0.16s`, and a string comparison
    // then fails on a leading zero while saying nothing about §11.6.
    const durations = [...css.matchAll(/(?<![\w.-])(\d+(?:\.\d+)?|\.\d+)(ms|s)(?![\w-])/g)].map(
      ([, amount, unit]) => (unit === 's' ? Number(amount) * 1000 : Number(amount)),
    );
    expect(durations.length).toBeGreaterThan(0);
    for (const ms of durations) expect([140, 160, 200]).toContain(ms);
    for (const [curve] of css.matchAll(/cubic-bezier\([^)]*\)|\b(?:ease-in-out|ease)\b/g)) {
      expect(curve.replace(/\s+/g, '').replace(/\b0\.(\d)/g, '.$1')).toBe(
        'cubic-bezier(.2,.85,.2,1)',
      );
    }
  });

  it('gives the grid ground its fallback, so a missing row is the default step', () => {
    expect(css).toContain('var(--cdt-tile, 186px)');
  });

  it("defines every class name the grid lane's components apply", () => {
    // The two halves of the shelf are built in separate lanes and meet here. A class one side
    // applies and the other never declares is invisible until both are merged.
    const contract = [
      'cdt-era-header',
      'cdt-era-chevron',
      'cdt-era-label',
      'cdt-era-summary',
      'cdt-era-flags',
      'cdt-shelf-grid',
      'cdt-shelf-grid-row',
      'cdt-shelf-spacer',
      'cdt-shelf-peek-slot',
      'cdt-reference-tail',
      'cdt-reference-head',
      'cdt-reference-row',
      'cdt-reference-summary',
      'cdt-attention-row',
      'cdt-attention-chip',
      // The 56th name. AttentionRow applied it before shelf.css declared it, so the broken
      // state selected nothing — the contract list is what makes that a failure and not a
      // silent one.
      'cdt-attention-chip--broken',
      'cdt-attention-count',
      'cdt-attention-label',
      'cdt-attention-sub',
      'cdt-attention-headline',
      'cdt-attention-hint',
      'cdt-list',
      'cdt-list-header',
      'cdt-list-row',
      'cdt-list-c1',
      'cdt-list-c2',
      'cdt-list-c3',
      'cdt-list-c4',
      'cdt-list-c5',
      'cdt-list-c6',
      'cdt-list-c7',
      'cdt-list-c8',
      'cdt-peek',
      'cdt-peek-head',
      'cdt-peek-rule',
      'cdt-peek-path',
      'cdt-peek-readme',
      'cdt-peek-commits',
      'cdt-peek-commit',
      'cdt-peek-observation',
      'cdt-peek-facts',
      'cdt-peek-fact-key',
      'cdt-peek-fact-value',
      'cdt-topbar--shed-1',
      'cdt-topbar--shed-2',
      'cdt-topbar--shed-3',
    ];
    const declared = new Set(
      [...css.matchAll(/\.((?:cdt-)[A-Za-z0-9_-]+)/g)].map(([, name]) => name),
    );
    expect(contract.filter((name) => !declared.has(name))).toEqual([]);
  });

  it('defines every class this plan’s own components apply', () => {
    const mine = [
      'cdt-shelf',
      'cdt-shelf-bar',
      'cdt-shelf-scroll',
      'cdt-shelf-wordmark',
      'cdt-shelf-wordmark-text',
      'cdt-shelf-mark',
      'cdt-shelf-mark-bar',
      'cdt-shelf-control',
      'cdt-shelf-control-key',
      'cdt-shelf-control-value',
      'cdt-shelf-seg',
      'cdt-shelf-seg-item',
      'cdt-shelf-glyph',
      'cdt-shelf-glyph-mark',
      'cdt-shelf-field',
      'cdt-shelf-field-input',
      'cdt-shelf-pill',
      'cdt-shelf-pill-label',
      'cdt-shelf-pill-drop',
      'cdt-shelf-notice-slot',
      'cdt-shelf-notice',
      'cdt-shelf-notice-title',
      'cdt-shelf-notice-body',
      'cdt-shelf-notice-actions',
      'cdt-shelf-notice-primary',
      'cdt-shelf-notice-secondary',
      'cdt-shelf-empty',
      'cdt-shelf-empty-heading',
      'cdt-shelf-empty-reason',
      'cdt-shelf-empty-actions',
      'cdt-shelf-empty-action',
    ];
    const declared = new Set(
      [...css.matchAll(/\.((?:cdt-)[A-Za-z0-9_-]+)/g)].map(([, name]) => name),
    );
    expect(mine.filter((name) => !declared.has(name))).toEqual([]);
  });

  it('left-aligns a short final row rather than centring it', () => {
    // Centring makes the whole grid appear to shift when a scan adds one project.
    expect(css).toMatch(/\.cdt-shelf-grid\s*\{[^}]*justify-content:\s*start/);
  });
});
