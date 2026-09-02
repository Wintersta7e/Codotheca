import { act, render, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ViewState } from '../../generated/protocol';
import type { ShelfView } from '../shelf/viewState';
import type { AppDeps } from './deps';
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
