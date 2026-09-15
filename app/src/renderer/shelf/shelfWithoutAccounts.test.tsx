/**
 * **AC-P2-20-9, the half neither of the other two tests can carry.**
 *
 * §20.11: *"The shelf is fully functional with zero accounts. No empty 'Not cloned' or 'Wishlist'
 * section with a *connect to see* placeholder — a section that exists only to advertise a feature
 * is an ad."*
 *
 * `notice.test.ts` tests a priority array and `firstRunNoAccount.test.ts` reads sources; **neither
 * renders a shelf**, so a shelf that threw on a missing account would pass both. This one mounts
 * the real component with no account anywhere in its inputs — which is the only state phase 2 can
 * be in until someone connects — and asserts what a user would see.
 */
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ProjectId, ProjectRow } from '../../generated/protocol.js';
import type { ShelfProps } from './Shelf.js';
import { SHELF_SCROLL_CLASS, Shelf } from './Shelf.js';
import { toShelfRow } from './row.js';
import { DEFAULT_SHELF_VIEW } from './viewState.js';

afterEach(cleanup);

function row(id: number, name: string): ProjectRow {
  return {
    id: id as ProjectId,
    name,
    seedBasename: name,
    designation: null,
    presence: 'present',
    isArchived: false,
    isReference: false,
    isBare: false,
    isShallow: false,
    hasSubmodules: false,
    primaryLocation: null,
    locationCount: 1,
    errorKind: null,
    errorAt: null,
    lastActivityAt: 1_759_000_000,
    lastUserCommitAt: null,
    firstCommitAt: null,
    trackedBytes: null,
    dirtyCount: null,
    ahead: null,
    behind: null,
    branch: null,
    conditionSignal: null,
    refstateObservedAt: null,
    worktreeObservedAt: null,
    languagePrimary: null,
    sceneHash: null,
    authoredByUser: null,
    playSeconds: 0,
    lastPlayedAt: null,
    noteCount: 0,
    collectionIds: [],
  } as unknown as ProjectRow;
}

/** One era section holding two real rows, so the shelf has something to paint. */
function page(): ShelfProps['page'] {
  const rows = [row(1, 'alpha'), row(2, 'beta')].map((r) => toShelfRow(r));
  return {
    sections: [
      {
        id: 'era:live',
        order: 0,
        year: null,
        cutAgainstYear: 2026,
        label: 'LIVE',
        agg: {
          count: rows.length,
          trackedBytes: 0,
          indexedCount: rows.length,
          unpushed: 0,
          uncommitted: 0,
          interrupted: 0,
          unchecked: 0,
        },
        rows,
      },
    ],
    reference: [],
    ignored: [],
    matched: rows.length,
    renderedTotal: rows.length,
    orderKey: 'k',
    generation: 1,
    ast: DEFAULT_SHELF_VIEW.ast,
  };
}

const props = (over: Partial<ShelfProps> = {}): ShelfProps => ({
  view: DEFAULT_SHELF_VIEW,
  page: page(),
  counts: {
    matched: 2,
    total: 2,
    reference: 0,
    classified: 2,
    classificationKnown: true,
  },
  // No account exists, so no account-derived notice qualifies.
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
  children: <div data-testid="body">rows</div>,
  ...over,
});

describe('the shelf with zero accounts', () => {
  it('AC-P2-20-9 mounts and paints without an account anywhere in its inputs', () => {
    const { container } = render(<Shelf {...props()} />);
    // It painted: the bar, exactly one scroll container, and the body inside it.
    expect(container.querySelector('.cdt-shelf-bar')).toBeTruthy();
    expect(container.querySelectorAll(`.${SHELF_SCROLL_CLASS}`)).toHaveLength(1);
    expect(screen.getByTestId('body')).toBeTruthy();
  });

  it('advertises nothing: no section exists whose only content is a connect placeholder', () => {
    const { container } = render(<Shelf {...props()} />);
    const text = container.textContent ?? '';
    expect(text.length, 'the shelf rendered nothing, so this proved nothing').toBeGreaterThan(0);
    // A section that exists only to advertise a feature is an ad.
    for (const advert of [/connect to see/iu, /not cloned/iu, /wishlist/iu, /sign in/iu]) {
      expect(advert.test(text), `the shelf advertises: ${advert.source}`).toBe(false);
    }
  });

  it('annotates no control with account state, and disables none', () => {
    const { container } = render(<Shelf {...props()} />);
    const disabled = [...container.querySelectorAll('button')].filter((b) => b.disabled);
    expect(disabled, 'account state disabled a control').toEqual([]);
    const text = container.textContent ?? '';
    // Play is never delayed or annotated by account state, so no control may say so.
    expect(/requires? (?:a )?(?:github|account|token)/iu.test(text)).toBe(false);
  });

  it('draws no notice at all when nothing qualifies, offer included', () => {
    // The connect offer is a *candidate*, not something the shelf raises on its own: with no
    // notices supplied there is no slot, which is what "exactly one, dismissible" rests on.
    const { container } = render(<Shelf {...props()} />);
    expect(container.querySelector('.cdt-shelf-notice-slot')).toBeNull();
  });
});
