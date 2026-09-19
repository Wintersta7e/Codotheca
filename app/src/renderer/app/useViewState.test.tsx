import { act, render, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { HealthState, HealthSummary, ProjectId, ViewState } from '../../generated/protocol';
import { toShelfRow, type ShelfRow } from '../shelf/row';
import type { ShelfView } from '../shelf/viewState';
import { makeProjectRow } from '../testing/projectRow';
import type { AppDeps } from './deps';
import { ShelfScreen } from './ShelfScreen';
import { fakeAppDeps, type FakeAppDeps } from './testDeps';
import { useViewState } from './useViewState';

const DEBOUNCE_MS = 400;

const stored: ViewState = {
  query: 'is:dirty',
  sort: 'name',
  viewMode: 'list',
  density: 148,
  collapsedSections: ['era:2019'],
  scrollOffset: 120,
  selectedProjectId: null,
  dismissedNotices: [],
  windowGeometry: null,
  savedAt: 1_700_000_000,
};

function Probe({
  deps,
  seen,
}: {
  deps: AppDeps;
  seen: [ShelfView, (next: ShelfView) => void][];
}): ReactElement {
  seen.push(useViewState(deps, DEBOUNCE_MS));
  return <div />;
}

function mount(fake: FakeAppDeps): {
  last: () => [ShelfView, (next: ShelfView) => void];
  writes: () => unknown[];
} {
  const seen: [ShelfView, (next: ShelfView) => void][] = [];
  render(<Probe deps={fake.deps} seen={seen} />);
  return {
    last: () => {
      const state = seen.at(-1);
      if (state === undefined) throw new Error('the hook rendered nothing');
      return state;
    },
    writes: () => fake.calls.filter((call) => call.name === 'view.set').map((call) => call.args),
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe('useViewState', () => {
  it('reads the stored view once and parses the query with it', async () => {
    const fake = fakeAppDeps({ 'view.get': () => stored, 'view.set': () => ({}) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last()[0].query).toBe('is:dirty');
    });
    expect(view.last()[0].sort).toBe('name');
    expect(view.last()[0].viewMode).toBe('list');
    expect(view.last()[0].density).toBe(148);
    expect(view.last()[0].collapsed.get('era:2019')).toBe(true);
    // The AST travels with the string, so the pills and the filter cannot come from two
    // different readings of one query.
    expect(view.last()[0].ast.terms.length).toBeGreaterThan(0);
    expect(fake.calls.filter((call) => call.name === 'view.get')).toHaveLength(1);
  });

  it('an idle shelf issues no view.set at all', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const fake = fakeAppDeps({ 'view.get': () => stored, 'view.set': () => ({}) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last()[0].query).toBe('is:dirty');
    });

    // A full round trip and nothing the user did. `patchFor` returns null when nothing changed,
    // and this is the assertion that the return value is acted on rather than hoped for.
    act(() => {
      vi.advanceTimersByTime(DEBOUNCE_MS * 4);
    });
    expect(view.writes()).toHaveLength(0);
  });

  it('writes one patch per settled change, carrying only what changed', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const fake = fakeAppDeps({ 'view.get': () => stored, 'view.set': () => ({}) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last()[0].query).toBe('is:dirty');
    });

    act(() => {
      view.last()[1]({ ...view.last()[0], sort: 'size' });
    });
    expect(view.writes()).toHaveLength(0);
    act(() => {
      vi.advanceTimersByTime(DEBOUNCE_MS);
    });

    expect(view.writes()).toHaveLength(1);
    const patch = (view.writes()[0] as { patch: Record<string, unknown> }).patch;
    expect(patch['sort']).toBe('size');
    expect(patch['query']).toBeNull();
    expect(patch['viewMode']).toBeNull();
    // §11.2 gives geometry to the shell; the renderer has no reading of it to write back.
    expect(patch['windowGeometry']).toBeNull();
  });

  it('coalesces a burst into one write against the last persisted view', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const fake = fakeAppDeps({ 'view.get': () => stored, 'view.set': () => ({}) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last()[0].query).toBe('is:dirty');
    });

    act(() => {
      view.last()[1]({ ...view.last()[0], query: 'is:d' });
    });
    act(() => {
      vi.advanceTimersByTime(DEBOUNCE_MS / 2);
    });
    act(() => {
      view.last()[1]({ ...view.last()[0], query: 'is:dir' });
    });
    act(() => {
      vi.advanceTimersByTime(DEBOUNCE_MS);
    });

    expect(view.writes()).toHaveLength(1);
    expect((view.writes()[0] as { patch: Record<string, unknown> }).patch['query']).toBe('is:dir');
  });

  it('a change and a change back settles to no write', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const fake = fakeAppDeps({ 'view.get': () => stored, 'view.set': () => ({}) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last()[0].query).toBe('is:dirty');
    });

    const original = view.last()[0];
    act(() => {
      view.last()[1]({ ...original, sort: 'size' });
    });
    act(() => {
      view.last()[1](original);
    });
    act(() => {
      vi.advanceTimersByTime(DEBOUNCE_MS * 2);
    });
    expect(view.writes()).toHaveLength(0);
  });

  /**
   * [p3] `AC-P3-35-6`'s command-stream half, §35.5.
   *
   * The claim is about the **pair**: `ShelfScreen` resolves a stored key the projection cannot
   * honour at the point the comparator is chosen, and this hook is the one issuer of `view.set`
   * (`useViewState.ts:35-38`, over `patchFor`). Asserting it on the hook alone would prove only
   * that an idle shelf writes nothing, which the case above already says — so the screen is
   * mounted behind the real hook and the fake's command stream is read.
   */
  describe('AC-P3-35-6 the key is not offered when nothing carries a reading, and nothing is written back', () => {
    const NOW = Math.floor(Date.UTC(2026, 6, 1, 12) / 1000);
    const storedNeedsAttention: ViewState = { ...stored, query: '', sort: 'needs_attention' };

    const reading = (state: HealthState, scoredOpen: number | null): HealthSummary => ({
      state,
      scoredOpen,
      unverified: null,
      unknownChecks: null,
      observedAt: null,
    });

    function mountShelf(rows: readonly ShelfRow[]): {
      last: () => [ShelfView, (next: ShelfView) => void];
      writes: () => unknown[];
      sortValue: () => string | null;
    } {
      const fake = fakeAppDeps(
        { 'view.get': () => storedNeedsAttention, 'view.set': () => ({}) },
        { effectsTier: 'off' },
      );
      fake.setNow(NOW);
      const seen: [ShelfView, (next: ShelfView) => void][] = [];
      function Probe(): ReactElement {
        const pair = useViewState(fake.deps, DEBOUNCE_MS);
        seen.push(pair);
        return (
          <ShelfScreen
            deps={fake.deps}
            rows={rows}
            library="present"
            problems={null}
            generation={7}
            view={pair[0]}
            onViewChange={pair[1]}
            notices={[]}
            scan={null}
            sessions={new Map()}
            firstRunCompletedAt={null}
            tier="off"
            onOpenProject={vi.fn()}
            onOpenPalette={vi.fn()}
            onOpenSettings={vi.fn()}
            onOpenScanSummary={vi.fn()}
            onAddScanRoot={vi.fn()}
          />
        );
      }
      // Scoped to this mount's own container: this file's `afterEach` does not call `cleanup`,
      // so an earlier render's bar is still in `document` and a document-wide query reads it.
      const { container } = render(<Probe />);
      return {
        last: () => {
          const state = seen.at(-1);
          if (state === undefined) throw new Error('the hook rendered nothing');
          return state;
        },
        writes: () =>
          fake.calls.filter((call) => call.name === 'view.set').map((call) => call.args),
        sortValue: () =>
          container.querySelector('[data-slot="sort"] .cdt-shelf-control-value')?.textContent ??
          null,
      };
    }

    it('renders in the default order and issues no view.set when no row carries a reading', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const rows = [1, 2].map((id) =>
        toShelfRow(makeProjectRow({ id: id as ProjectId, lastTouchedAt: NOW - id * 86_400 })),
      );
      const view = mountShelf(rows);
      await waitFor(() => {
        expect(view.last()[0].sort).toBe('needs_attention');
      });

      // The control does not offer it, so the bar renders the resolved default.
      expect(view.sortValue()).toBe('LAST TOUCHED');
      // And the stored preference is kept: it is not deleted because today's library cannot
      // honour it. Asserted on the command stream, never by reading the stored value back.
      act(() => {
        vi.advanceTimersByTime(DEBOUNCE_MS * 4);
      });
      expect(view.writes()).toHaveLength(0);
    });

    it('offers the key again the moment one row carries a reading, still writing nothing', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const rows = [
        toShelfRow(
          makeProjectRow({
            id: 1 as ProjectId,
            lastTouchedAt: NOW - 86_400,
            healthSummary: reading('live', 3),
          }),
        ),
        toShelfRow(makeProjectRow({ id: 2 as ProjectId, lastTouchedAt: NOW - 2 * 86_400 })),
      ];
      const view = mountShelf(rows);
      await waitFor(() => {
        expect(view.last()[0].sort).toBe('needs_attention');
      });

      expect(view.sortValue()).toBe('NEEDS ATTENTION');
      act(() => {
        vi.advanceTimersByTime(DEBOUNCE_MS * 4);
      });
      // Nothing changed, so nothing is written on the way back either.
      expect(view.writes()).toHaveLength(0);
    });
  });

  it('falls back to the default view when the core refuses view.get', async () => {
    const fake = fakeAppDeps({
      'view.get': () => {
        throw new Error('core is down');
      },
      'view.set': () => ({}),
    });
    const view = mount(fake);
    await waitFor(() => {
      expect(fake.calls.filter((call) => call.name === 'view.get')).toHaveLength(1);
    });
    expect(view.last()[0].query).toBe('');
    expect(view.last()[0].density).toBe(186);
  });
});
