import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { LocationId, ProjectId, ProjectRow } from '../../generated/protocol.js';
import { makeLocationRef, makeProjectRow } from '../testing/projectRow.js';
import { QuickSwitch } from './QuickSwitch.js';
import type { QuickSwitchProps } from './QuickSwitch.js';
// The dom project processes the renderer's stylesheets so `?raw` returns the real text; without
// that it is stubbed to an empty module and every assertion below passes on nothing.
import css from './quickSwitch.css?raw';

afterEach(cleanup);

const rows: ProjectRow[] = [
  makeProjectRow({
    id: 1 as ProjectId,
    name: 'Nightfall',
    primaryLanguage: 'Rust',
    branch: 'main',
    conditionSignal: 'dormant',
    lastTouchedAt: 1_800_000_000 - 400 * 86_400,
    primaryLocation: makeLocationRef(11),
  }),
  makeProjectRow({
    id: 2 as ProjectId,
    name: 'Offshore',
    conditionSignal: null,
    lastTouchedAt: 1_800_000_000 - 800 * 86_400,
    primaryLocation: null,
    presence: null,
  }),
];

function view(over: Partial<QuickSwitchProps> = {}): {
  props: QuickSwitchProps;
} & ReturnType<typeof render> {
  const props: QuickSwitchProps = {
    rows,
    query: '',
    cursor: 0,
    liveSessionProjectIds: new Set<ProjectId>(),
    nowSecs: 1_800_000_000,
    effectsTier: 'full',
    jewelFor: () => 'oklch(0.6 0.17 26)',
    onQueryChange: vi.fn(),
    onPoint: vi.fn(),
    onLaunch: vi.fn(),
    onOpenPage: vi.fn(),
    onClose: vi.fn(),
    onKeyDown: vi.fn(),
    ...over,
  };
  return { props, ...render(<QuickSwitch {...props} />) };
}

const listbox = (): HTMLElement => {
  const node = document.getElementById('qs-listbox');
  if (node === null) throw new Error('the listbox did not render');
  return node;
};

describe('QuickSwitch', () => {
  it('is a listbox whose active option follows the cursor', () => {
    view({ cursor: 1 });
    const input = screen.getByRole('combobox');
    const options = screen.getAllByRole('option');
    expect(options).toHaveLength(2);
    expect(options[1]?.getAttribute('aria-selected')).toBe('true');
    expect(input.getAttribute('aria-activedescendant')).toBe(options[1]?.id);
  });

  it('states the full match set in the query bar, not the capped list', () => {
    const many = Array.from({ length: 44 }, (_, i) =>
      makeProjectRow({
        id: (i + 1) as ProjectId,
        name: `hit-${String(i)}`,
        lastTouchedAt: 900 - i,
      }),
    );
    view({ rows: many, query: 'hit' });
    expect(screen.getByText('44 OF 44')).toBeTruthy();
    expect(screen.getAllByRole('option')).toHaveLength(40);
  });

  // Criterion 45b: no rank glyph in the palette row's leading slot — assert the node, not a digit.
  it('draws a condition dot and never a rank glyph in the leading slot', () => {
    view();
    const slots = document.querySelectorAll('.qs-dot-slot');
    expect(slots).toHaveLength(2);
    expect(slots[0]?.querySelectorAll('.qs-dot')).toHaveLength(1);
    expect(slots[0]?.textContent).toBe('');
    // §5.4a: condition_signal IS NULL draws no dot at all.
    expect(slots[1]?.childElementCount).toBe(0);
    expect(screen.getByLabelText('Condition: dormant')).toBeTruthy();
  });

  it('shows ↵ LAUNCH on the selected row and the reason when there is no copy here', () => {
    // Scoped to the listbox: the footer legend carries the same words, and an unscoped query
    // would find it and pass whatever the row does.
    const { unmount } = view({ cursor: 0 });
    expect(within(listbox()).getByText('↵ LAUNCH')).toBeTruthy();
    unmount();
    view({ cursor: 1 });
    expect(within(listbox()).getByText('NO COPY ON THIS MACHINE')).toBeTruthy();
    expect(within(listbox()).queryByText('↵ LAUNCH')).toBeNull();
  });

  it('launches the selected row against its primary location and closes', () => {
    const { props } = view({ cursor: 0 });
    fireEvent.click(screen.getAllByRole('option')[0] as HTMLElement);
    expect(props.onLaunch).toHaveBeenCalledWith(1 as ProjectId, 11 as LocationId);
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it('opens the page instead of leaving a dead row when there is no local copy', () => {
    const { props } = view({ cursor: 1 });
    fireEvent.click(screen.getAllByRole('option')[1] as HTMLElement);
    expect(props.onLaunch).not.toHaveBeenCalled();
    expect(props.onOpenPage).toHaveBeenCalledWith(2 as ProjectId);
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it('closes on a backdrop click and not on a click inside the panel', () => {
    const { props } = view();
    fireEvent.click(document.querySelector('.qs-panel') as HTMLElement);
    expect(props.onClose).not.toHaveBeenCalled();
    fireEvent.click(document.querySelector('.qs-backdrop') as HTMLElement);
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  it('renders §8.6’s empty state as the panel’s entire content', () => {
    view({ rows: [], query: 'zzz' });
    expect(screen.getByText('No project matches that.')).toBeTruthy();
    expect(screen.queryAllByRole('option')).toHaveLength(0);
    expect(screen.getByText('0 OF 0')).toBeTruthy();
  });

  it('staggers rows at full and flattens the ladder at off', () => {
    const { unmount } = view();
    const first = document.querySelectorAll<HTMLElement>('.qs-row');
    expect(first[1]?.style.getPropertyValue('--cdt-qs-row-delay')).toBe('14ms');
    unmount();
    view({ effectsTier: 'off' });
    expect(
      document
        .querySelectorAll<HTMLElement>('.qs-row')[1]
        ?.style.getPropertyValue('--cdt-qs-row-delay'),
    ).toBe('0ms');
  });

  // Criterion 54: a DOM audit over quick switch finds zero roast blocks.
  it('renders no roast block and no description', () => {
    view({
      rows: [makeProjectRow({ id: 9 as ProjectId, name: 'x', description: 'a description' })],
    });
    expect(screen.queryByText('a description')).toBeNull();
    expect(document.querySelectorAll('[data-roast]')).toHaveLength(0);
  });

  it('carries §11.7’s four footer legends verbatim', () => {
    view();
    for (const hint of ['↑↓ MOVE', '↵ LAUNCH', '⇧↵ OPEN THE PAGE', 'ESC']) {
      expect(screen.getAllByText(hint).length).toBeGreaterThan(0);
    }
  });

  // R36's shape one level down: a clamp whose selector names a class the panel does not render
  // stops clamping and no gate says so. The stylesheet's text alone cannot show that; the
  // rendered DOM can.
  it('every tier clamp selects a class the panel actually renders', () => {
    expect(css.length, 'the stylesheet was stubbed to an empty module').toBeGreaterThan(0);
    const body = css.replace(/\/\*[\s\S]*?\*\//g, '');
    const tierSelectors = [...body.matchAll(/([^{}]*\[data-effects-tier[^{}]*)\{/g)].map(
      (m) => m[1] ?? '',
    );
    expect(tierSelectors.length).toBeGreaterThan(0);
    const classes = new Set<string>();
    for (const selector of tierSelectors) {
      for (const [, name] of selector.matchAll(/\.([a-z][\w-]*)/g)) classes.add(name ?? '');
    }
    expect(classes.size).toBeGreaterThan(0);
    view();
    for (const name of classes) {
      expect(document.querySelectorAll(`.${name}`).length, name).toBeGreaterThan(0);
    }
  });
});

/**
 * AC-P2-23-6, asserted **by rendering the row** (§23.9), not by reading `paletteSubLine`'s
 * inputs. For a zero-location project `lastTouchedAt` is the `created_at` fallback — the moment
 * Codotheca wrote the row — so §8.6's tail would read `opened this month` about a repository
 * that has never been on this machine. That is the phase-1 defect §19.7 records.
 */
describe('§23.4: the palette sub-line claims no interaction it never had', () => {
  const subLineOf = (name: string): string => {
    const row = within(listbox()).getByText(name).closest('.qs-row');
    if (row === null) throw new Error(`no row for ${name}`);
    const sub = row.querySelector('.qs-sub');
    if (sub === null) throw new Error(`no sub-line for ${name}`);
    return sub.textContent ?? '';
  };

  it('renders no tail for a not-cloned row, for any value of the clock', () => {
    const NOW_SECS = 1_800_000_000;
    for (const touched of [
      NOW_SECS,
      NOW_SECS - 86_400,
      NOW_SECS - 29 * 86_400,
      NOW_SECS - 400 * 86_400,
      0,
    ]) {
      view({
        rows: [
          makeProjectRow({
            id: 2 as ProjectId,
            name: 'Offshore',
            primaryLanguage: 'Rust',
            branch: 'main',
            primaryLocation: null,
            presence: null,
            lastTouchedAt: touched,
          }),
        ],
        nowSecs: NOW_SECS,
      });
      const sub = subLineOf('Offshore');
      expect(sub, `tail leaked at lastTouchedAt=${String(touched)}`).not.toContain(
        'opened this month',
      );
      expect(sub).not.toContain('months cold');
      expect(sub).not.toContain('in session');
      // The remaining fields still render: a hydrated not-cloned project legitimately has a
      // language and a branch, and §23 invents no fourth tail word for the gap.
      expect(sub).toBe('.rs · main');
      cleanup();
    }
  });

  it('leaves a located row byte-identical', () => {
    // The fix is a regression wearing a fix's clothes if this moves.
    view();
    expect(subLineOf('Nightfall')).toBe('.rs · main · 13 months cold');
  });

  it('renders the tail for a not-cloned row that is somehow in session — it does not', () => {
    // `in session` is the one tail word a not-cloned project could reach through a different
    // input, so it is asserted explicitly rather than left to the clock spread above.
    view({
      rows: [
        makeProjectRow({
          id: 2 as ProjectId,
          name: 'Offshore',
          primaryLocation: null,
          presence: null,
        }),
      ],
      liveSessionProjectIds: new Set<ProjectId>([2 as ProjectId]),
    });
    expect(subLineOf('Offshore')).not.toContain('in session');
  });
});
