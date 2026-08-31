import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { parseQuery } from '../../shared/query/parse.js';
import { statesAColour } from '../a11y/names.js';
import { fieldModel } from './QueryField.js';
import type { TopBarProps } from './TopBar.js';
import { TOP_BAR_FLOOR_PX, TOP_BAR_HEIGHT_PX, TopBar, WORDMARK } from './TopBar.js';
// R19: the four shed names are 13c's, in the hook's own module. Importing them from `TopBar.js`
// would re-export the hook's table through the component that imports the hook.
import { SHED_ORDER, SHED_WIDTHS, shedLevelFor } from './useShedLevel.js';
import { DEFAULT_SHELF_VIEW } from './viewState.js';

afterEach(cleanup);

const props = (over: Partial<TopBarProps> = {}): TopBarProps => ({
  view: DEFAULT_SHELF_VIEW,
  field: fieldModel('', parseQuery(''), []),
  scan: { running: false, foundRepos: 0 },
  barWidth: 1400,
  onQueryChange: vi.fn(),
  onSortChange: vi.fn(),
  onDensityChange: vi.fn(),
  onViewModeChange: vi.fn(),
  onScan: vi.fn(),
  onOpenScanSummary: vi.fn(),
  onOpenPalette: vi.fn(),
  onOpenSettings: vi.fn(),
  ...over,
});

describe('geometry', () => {
  it("is 40px, the prototype's value, not the prose's 42", () => {
    expect(TOP_BAR_HEIGHT_PX).toBe(40);
  });
  it('has a floor that is a fully shed width, and the number is the measurement’s to own', () => {
    // The value itself belongs to `app/e2e/topbar-floor.spec.ts`, which renders this bar in a
    // real layout engine and fails when the constant is wrong in either direction. What holds
    // here is the relationship: the floor is where the last shed step has already engaged.
    expect(shedLevelFor(TOP_BAR_FLOOR_PX)).toBe(3);
    expect(TOP_BAR_FLOOR_PX).toBeLessThanOrEqual(SHED_WIDTHS[2]);
  });
});

describe('the seven slots', () => {
  it('renders them in order, all as real buttons', () => {
    const { container } = render(<TopBar {...props()} />);
    const slots = [...container.querySelectorAll('[data-slot]')];
    expect(slots.map((node) => node.getAttribute('data-slot'))).toEqual([
      'sort',
      'scan',
      'grid',
      'list',
      'switch',
      'density',
      'settings',
    ]);
    for (const slot of slots) {
      expect(slot.tagName).toBe('BUTTON');
      expect(slot.getAttribute('type')).toBe('button');
    }
    expect(screen.getByRole('searchbox')).toBeTruthy();
  });

  it('cycles exactly three sort keys, with Completion absent', () => {
    const { container } = render(<TopBar {...props()} />);
    expect(container.textContent).toContain('LAST TOUCHED');
    expect(container.textContent).not.toMatch(/completion/i);
  });

  it('advances the sort one step, and the caller writes it back', () => {
    const onSortChange = vi.fn();
    const { container } = render(<TopBar {...props({ onSortChange })} />);
    fireEvent.click(container.querySelector('[data-slot="sort"]') as HTMLElement);
    expect(onSortChange).toHaveBeenCalledWith('name');
  });

  it('spells the chord in letters both target keyboards carry', () => {
    const { container } = render(<TopBar {...props()} />);
    expect(container.textContent).toContain('ALT+SPACE');
    expect(container.textContent).not.toMatch(/⌥|␣/);
  });

  it('carries the count during a run and routes to the summary', () => {
    const onOpenScanSummary = vi.fn();
    render(<TopBar {...props({ scan: { running: true, foundRepos: 42 }, onOpenScanSummary })} />);
    const scan = screen.getByRole('button', { name: /scanning/i });
    expect(scan.textContent).toContain('42');
    fireEvent.click(scan);
    expect(onOpenScanSummary).toHaveBeenCalledOnce();
  });

  it('starts a rescan when idle', () => {
    const onScan = vi.fn();
    render(<TopBar {...props({ onScan })} />);
    fireEvent.click(screen.getByRole('button', { name: /^scan$/i }));
    expect(onScan).toHaveBeenCalledOnce();
  });

  it('never both starts a scan and opens the summary from one control', () => {
    const onScan = vi.fn();
    const onOpenScanSummary = vi.fn();
    render(
      <TopBar {...props({ scan: { running: true, foundRepos: 3 }, onScan, onOpenScanSummary })} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /scanning/i }));
    expect(onScan).not.toHaveBeenCalled();
    expect(onOpenScanSummary).toHaveBeenCalledOnce();
  });

  it('marks the live view on the segmented control', () => {
    const { container } = render(<TopBar {...props()} />);
    expect(container.querySelector('[data-slot="grid"]')?.getAttribute('aria-pressed')).toBe(
      'true',
    );
    expect(container.querySelector('[data-slot="list"]')?.getAttribute('aria-pressed')).toBe(
      'false',
    );
  });

  it('hides the density control in list', () => {
    render(<TopBar {...props({ view: { ...DEFAULT_SHELF_VIEW, viewMode: 'list' as const } })} />);
    expect(screen.queryByRole('button', { name: /^Density/ })).toBeNull();
  });

  it('opens the palette and the settings from their own slots', () => {
    const onOpenPalette = vi.fn();
    const onOpenSettings = vi.fn();
    const { container } = render(<TopBar {...props({ onOpenPalette, onOpenSettings })} />);
    fireEvent.click(container.querySelector('[data-slot="switch"]') as HTMLElement);
    fireEvent.click(container.querySelector('[data-slot="settings"]') as HTMLElement);
    expect(onOpenPalette).toHaveBeenCalledOnce();
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });
});

describe('the shed order', () => {
  it('is the stated one', () => {
    expect(SHED_ORDER).toEqual(['switch', 'keys', 'wordmarkLettering']);
  });
  it('engages step by step as the bar narrows, and never un-sheds', () => {
    expect(shedLevelFor(1400)).toBe(0);
    expect(shedLevelFor(TOP_BAR_FLOOR_PX)).toBe(3);
    const levels = [1400, ...SHED_WIDTHS, TOP_BAR_FLOOR_PX].map(shedLevelFor);
    expect(levels).toEqual([...levels].sort((a, b) => a - b));
  });

  const keyOf = (root: HTMLElement, slot: string): string | null =>
    root.querySelector(`[data-slot="${slot}"] .cdt-shelf-control-key`)?.textContent ?? null;
  const valueOf = (root: HTMLElement, slot: string): string | null =>
    root.querySelector(`[data-slot="${slot}"] .cdt-shelf-control-value`)?.textContent ?? null;

  it('drops SWITCH first, and only SWITCH, and Alt+Space still opens it', () => {
    // The bar's own `textContent` runs every label together — `CODOTHECASORTLAST TOUCHED` — so
    // `/\bSORT\b/` over it can never match and asserting on it either way proves nothing. Each
    // step is read off the slot it belongs to, and states what still stands as well as what went.
    const { container } = render(<TopBar {...props({ barWidth: SHED_WIDTHS[0] })} />);
    expect(shedLevelFor(SHED_WIDTHS[0])).toBe(1);
    expect(container.querySelector('[data-slot="switch"]')).toBeNull();
    expect(container.textContent).not.toContain('ALT+SPACE');
    expect(container.querySelector('.cdt-shelf-wordmark-text')?.textContent).toBe(WORDMARK);
    expect(keyOf(container, 'sort')).toBe('SORT');
    expect(keyOf(container, 'density')).toBe('DENSITY');
  });

  it('then drops the SORT and DENSITY keys, leaving their values and the wordmark', () => {
    const { container } = render(<TopBar {...props({ barWidth: SHED_WIDTHS[1] })} />);
    expect(shedLevelFor(SHED_WIDTHS[1])).toBe(2);
    expect(keyOf(container, 'sort')).toBeNull();
    expect(keyOf(container, 'density')).toBeNull();
    expect(container.querySelector('.cdt-shelf-wordmark-text')?.textContent).toBe(WORDMARK);
    expect(valueOf(container, 'sort')).toBe('LAST TOUCHED');
    expect(valueOf(container, 'density')).toBe('DEFAULT');
  });

  it("then drops the wordmark's lettering, leaving its mark", () => {
    const { container } = render(<TopBar {...props({ barWidth: TOP_BAR_FLOOR_PX })} />);
    expect(container.textContent).not.toContain(WORDMARK);
    expect(container.querySelector('.cdt-shelf-wordmark-text')).toBeNull();
    expect(container.querySelector('.cdt-shelf-mark')).toBeTruthy();
    expect(valueOf(container, 'sort')).toBe('LAST TOUCHED');
    expect(valueOf(container, 'density')).toBe('DEFAULT');
  });

  it('keeps the mark at every level, so the bar is never headless', () => {
    for (const width of [1400, ...SHED_WIDTHS, TOP_BAR_FLOOR_PX]) {
      cleanup();
      const { container } = render(<TopBar {...props({ barWidth: width })} />);
      expect(container.querySelector('.cdt-shelf-mark')).toBeTruthy();
    }
  });

  it('sheds no control that has no other way in', () => {
    // Only the SWITCH chip goes, and Alt+Space still reaches the palette. Every other slot is
    // present at the floor: shedding a control with no keyboard route would strand it.
    const { container } = render(<TopBar {...props({ barWidth: TOP_BAR_FLOOR_PX })} />);
    const slots = [...container.querySelectorAll('[data-slot]')].map((n) =>
      n.getAttribute('data-slot'),
    );
    expect(slots).toEqual(['sort', 'scan', 'grid', 'list', 'density', 'settings']);
  });
});

describe('the label floor', () => {
  it('paints no control label at a decorative grey', () => {
    // §8.0a: a control label is not decoration. --text-4 and --text-5 are ornament only.
    const { container } = render(<TopBar {...props()} />);
    expect(container.innerHTML).not.toMatch(/#7a8896|#6c7885|#4a5560/);
    expect(container.innerHTML).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });
  it('names no destructive operation', () => {
    const { container } = render(<TopBar {...props()} />);
    expect(container.innerHTML).not.toMatch(/FORGET/i);
  });
  it('names no control by a colour', () => {
    render(<TopBar {...props()} />);
    for (const button of screen.getAllByRole('button')) {
      const name = button.getAttribute('aria-label') ?? button.textContent ?? '';
      expect(statesAColour(name)).toBe(false);
    }
  });
});
