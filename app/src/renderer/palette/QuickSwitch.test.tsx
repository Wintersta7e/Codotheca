import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { LocationId, ProjectId, ProjectRow } from '../../generated/protocol.js';
import { makeLocationRef, makeProjectRow } from '../testing/projectRow.js';
import { QuickSwitch } from './QuickSwitch.js';
import type { QuickSwitchProps } from './QuickSwitch.js';
// The dom project processes the renderer's stylesheets so `?raw` returns the real text; without
// that it is stubbed to an empty module and every assertion below passes on nothing.
import css from './quickSwitch.css?raw';
import { required } from '../../shared/required.js';

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

  it('shows ↵ LAUNCH on the selected row and ↵ INSTALL… when the project is not cloned', () => {
    // Scoped to the listbox: the footer legend carries the same words, and an unscoped query
    // would find it and pass whatever the row does.
    const { unmount } = view({ cursor: 0 });
    expect(within(listbox()).getByText('↵ LAUNCH')).toBeTruthy();
    unmount();
    view({ cursor: 1 });
    expect(within(listbox()).getByText('↵ INSTALL…')).toBeTruthy();
    expect(within(listbox()).queryByText('↵ LAUNCH')).toBeNull();
  });

  // §24.5 keeps `unavailable` its own words for the other fact: copies exist and none can be
  // opened. It is a different row from the not-cloned one and renders differently.
  it('states the reason when every copy is offline, and offers no INSTALL for it', () => {
    view({
      cursor: 0,
      rows: [
        makeProjectRow({
          id: 3 as ProjectId,
          name: 'Faraway',
          primaryLocation: makeLocationRef(13),
          presence: 'offline',
        }),
      ],
    });
    expect(within(listbox()).getByText('NO COPY ON THIS MACHINE')).toBeTruthy();
    expect(within(listbox()).queryByText('↵ INSTALL…')).toBeNull();
    expect(within(listbox()).queryByText('↵ LAUNCH')).toBeNull();
  });

  it('launches the selected row against its primary location and closes', () => {
    const { props } = view({ cursor: 0 });
    fireEvent.click(required(screen.getAllByRole('option')[0], 'first option'));
    expect(props.onLaunch).toHaveBeenCalledWith(1 as ProjectId, 11 as LocationId);
    expect(props.onClose).toHaveBeenCalledTimes(1);
  });

  /**
   * AC-P2-24-12's first half. **The palette does not clone**: what is asserted is that nothing
   * crossed the one channel a clone could travel on, not merely that navigation happened.
   * `request` is the renderer's whole vocabulary for reaching the core, so a zero there is the
   * absence of every install command rather than of one name someone remembered to exclude.
   */
  it('opens the page with Install asked for, and starts no clone', () => {
    const request = vi.fn(async () => Promise.resolve({ ok: true, value: {} }));
    const previous = (globalThis as { codotheca?: unknown }).codotheca;
    (globalThis as { codotheca?: unknown }).codotheca = { request };
    try {
      const { props } = view({ cursor: 1 });
      fireEvent.click(required(screen.getAllByRole('option')[1], 'second option'));
      expect(props.onLaunch).not.toHaveBeenCalled();
      expect(props.onOpenPage).toHaveBeenCalledWith(2 as ProjectId, 'install');
      expect(props.onClose).toHaveBeenCalledTimes(1);
      expect(request).not.toHaveBeenCalled();
    } finally {
      (globalThis as { codotheca?: unknown }).codotheca = previous;
    }
  });

  // The `unavailable` row keeps phase 1's behaviour exactly: the page opens and nothing is
  // focused, because there is nothing on it to install.
  it('opens the page with nothing asked for when every copy is offline', () => {
    const { props } = view({
      cursor: 0,
      rows: [
        makeProjectRow({
          id: 3 as ProjectId,
          name: 'Faraway',
          primaryLocation: makeLocationRef(13),
          presence: 'offline',
        }),
      ],
    });
    fireEvent.click(required(screen.getAllByRole('option')[0], 'first option'));
    expect(props.onLaunch).not.toHaveBeenCalled();
    expect(props.onOpenPage).toHaveBeenCalledWith(3 as ProjectId);
  });

  // §8.6's footer is byte-identical to phase 1's. It describes the majority case; a footer that
  // mutates per row is a moving target, and the row's trailing slot is the per-row channel.
  it('leaves §8.6’s footer byte-identical with a not-cloned row selected', () => {
    view({ cursor: 1 });
    const footer = document.querySelector('.qs-footer');
    if (footer === null) throw new Error('the footer did not render');
    expect([...footer.querySelectorAll('.qs-hint')].map((n) => n.textContent)).toEqual([
      '↑↓ MOVE',
      '↵ LAUNCH',
      '⇧↵ OPEN THE PAGE',
      'ESC',
    ]);
  });

  // It matched, so it counts. Excluding it would need a second denominator.
  it('counts a not-cloned row in <matched> OF <total>', () => {
    view();
    expect(screen.getByText('2 OF 2')).toBeTruthy();
    expect(screen.getAllByRole('option')).toHaveLength(2);
  });

  it('closes on a backdrop click and not on a click inside the panel', () => {
    const { props } = view();
    fireEvent.click(required(document.querySelector('.qs-panel'), 'palette panel'));
    expect(props.onClose).not.toHaveBeenCalled();
    fireEvent.click(required(document.querySelector('.qs-backdrop'), 'palette backdrop'));
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
      // language and a branch. §23 invented no fourth tail word and named §24 as the supplier;
      // §24.5 supplied it, and it is a fact rather than an interaction.
      expect(sub).toBe('.rs · main · not cloned');
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
