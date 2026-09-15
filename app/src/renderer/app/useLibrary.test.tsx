import { act, render, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';
import { describe, expect, it } from 'vitest';

import type { ProjectId, ProjectPage, ProjectRow } from '../../generated/protocol';
import { makeProjectRow } from '../testing/projectRow';
import type { AppDeps } from './deps';
import { fakeAppDeps, type FakeAppDeps } from './testDeps';
import { useLibrary, type LibraryState } from './useLibrary';

function page(rows: readonly ProjectRow[], generation = 3): ProjectPage {
  return {
    sections: [],
    rows: [...rows],
    window: { from: 0, to: rows.length },
    orderKey: 'k',
    generation,
  };
}

function Probe({ deps, seen }: { deps: AppDeps; seen: LibraryState[] }): ReactElement {
  const state = useLibrary(deps);
  seen.push(state);
  return <div data-testid="rows">{state.rows === null ? 'null' : String(state.rows.length)}</div>;
}

function mount(fake: FakeAppDeps): { seen: LibraryState[]; last: () => LibraryState } {
  const seen: LibraryState[] = [];
  render(<Probe deps={fake.deps} seen={seen} />);
  return {
    seen,
    last: () => {
      const state = seen.at(-1);
      if (state === undefined) throw new Error('the hook rendered nothing');
      return state;
    },
  };
}

const row = (id: number, over: Partial<ProjectRow> = {}): ProjectRow =>
  makeProjectRow({ id: id as ProjectId, name: `p${String(id)}`, ...over });

describe('useLibrary', () => {
  it('holds null before the first answer and an empty array after an empty one', async () => {
    // Two different sentences. `null` is *the core has not answered*; `[]` is *no projects*,
    // which is what drives §8.3a's empty state. Collapsing them renders unknown as zero.
    const fake = fakeAppDeps({ 'projects.list': () => page([]) });
    const view = mount(fake);
    expect(view.last().rows).toBeNull();

    await waitFor(() => {
      expect(view.last().rows).not.toBeNull();
    });
    expect(view.last().rows).toEqual([]);
    expect(view.last().rows).toHaveLength(0);
  });

  it('names the three states, so a surface taking a row list still receives the distinction', async () => {
    const fake = fakeAppDeps({ 'projects.list': () => page([]) });
    const view = mount(fake);
    // Before the answer. `[]` here would be §8.3a's empty state speaking for a read that has
    // not happened.
    expect(view.last().presence).toBe('uncomputed');

    await waitFor(() => {
      expect(view.last().presence).toBe('empty');
    });

    act(() => {
      view.last().store.upsert(row(1));
    });
    await waitFor(() => {
      expect(view.last().presence).toBe('present');
    });
  });

  it('a library that could not be read stays uncomputed rather than becoming empty', async () => {
    const fake = fakeAppDeps({
      'projects.list': () => {
        throw new Error('core is down');
      },
    });
    const view = mount(fake);
    await waitFor(() => {
      expect(fake.calls).toHaveLength(1);
    });
    expect(view.last().presence).toBe('uncomputed');
  });

  it('seeds from projects.list and takes the store generation, not a counter of its own', async () => {
    const fake = fakeAppDeps({ 'projects.list': () => page([row(1), row(2)], 7) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last().rows).toHaveLength(2);
    });
    expect(view.last().generation).toBe(7);
    expect(view.last().store.generation).toBe(7);
  });

  it('applies every projects event the topic declares', async () => {
    const fake = fakeAppDeps({ 'projects.list': () => page([row(1), row(2)], 1) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last().rows).toHaveLength(2);
    });

    // snapshot
    act(() => {
      fake.emit({
        topic: 'projects',
        event: 'snapshot',
        data: { epoch: 1, throughSeq: 1, generation: 9, rows: [row(1), row(2), row(3)] },
      });
    });
    expect(view.last().rows).toHaveLength(3);
    expect(view.last().generation).toBe(9);

    // upserted
    act(() => {
      fake.emit({
        topic: 'projects',
        event: 'upserted',
        data: { row: row(4, { name: 'four' }) },
      });
    });
    expect(view.last().rows?.map((r) => r.name)).toContain('four');

    // merged
    act(() => {
      fake.emit({ topic: 'projects', event: 'merged', data: { from: 3, into: 1 } });
    });
    expect(view.last().rows?.map((r) => r.id)).not.toContain(3);

    // flags_changed
    act(() => {
      fake.emit({
        topic: 'projects',
        event: 'flags_changed',
        data: { id: 1, isPinned: true, isArchived: false, isHidden: false },
      });
    });
    expect(view.last().rows?.find((r) => r.id === 1)?.isPinned).toBe(true);

    // condition_changed
    act(() => {
      fake.emit({
        topic: 'projects',
        event: 'condition_changed',
        data: { id: 1, conditionSignal: 'stale' },
      });
    });
    expect(view.last().rows?.find((r) => r.id === 1)?.conditionSignal).toBe('stale');
  });

  it('leaves art_ready to the card, which owns the swap', async () => {
    const fake = fakeAppDeps({ 'projects.list': () => page([row(1)], 1) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last().rows).toHaveLength(1);
    });
    const before = view.last().rows;

    act(() => {
      fake.emit({
        topic: 'projects',
        event: 'art_ready',
        data: { projectId: 1, sceneHash: 'abc', rendition: 'card', artState: 'ready' },
      });
    });
    // Same rows object: a re-render here would hold-and-swap every bitmap on the shelf, which
    // `useCardBitmap` already does for the one card that changed.
    expect(view.last().rows).toBe(before);
  });

  it('reload asks the core again', async () => {
    let generation = 1;
    const fake = fakeAppDeps({
      'projects.list': () => page([row(1)], (generation += 1)),
    });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last().generation).toBe(2);
    });

    act(() => {
      view.last().reload();
    });
    await waitFor(() => {
      expect(view.last().generation).toBe(3);
    });
    expect(fake.calls.filter((call) => call.name === 'projects.list')).toHaveLength(2);
  });

  it('re-reads the projection when a scan run ends, because no projects event announces a walk', async () => {
    // The shipped defect: `projects.list` ran once, at mount, over an empty library; the scan
    // then wrote every project and the shelf said `NOTHING INDEXED YET` until the app was
    // restarted. The `projects` topic never carries the walk's inserts.
    let rows: readonly ProjectRow[] = [];
    const fake = fakeAppDeps({ 'projects.list': () => page(rows, rows.length) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last().rows).toEqual([]);
    });

    rows = [row(1), row(2)];
    act(() => {
      fake.emit({ topic: 'scan', event: 'finished', data: {} });
    });
    await waitFor(() => {
      expect(view.last().rows).toHaveLength(2);
    });
    expect(fake.calls.filter((call) => call.name === 'projects.list')).toHaveLength(2);
  });

  it('re-reads on a cancelled run too, which still wrote every project it reached', async () => {
    let rows: readonly ProjectRow[] = [];
    const fake = fakeAppDeps({ 'projects.list': () => page(rows, rows.length) });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last().rows).toEqual([]);
    });

    rows = [row(4)];
    act(() => {
      fake.emit({ topic: 'scan', event: 'cancelled', data: {} });
    });
    await waitFor(() => {
      expect(view.last().rows).toHaveLength(1);
    });
  });

  it('does not re-read on a scan event that is not a run ending', async () => {
    // `progress` arrives many times a second on a large walk. A read per frame would put the
    // shelf's whole projection on the critical path of the scan it is watching.
    const fake = fakeAppDeps({ 'projects.list': () => page([]) });
    mount(fake);
    await waitFor(() => {
      expect(fake.calls).toHaveLength(1);
    });
    act(() => {
      fake.emit({ topic: 'scan', event: 'progress', data: { walkedDirs: 10 } });
      fake.emit({ topic: 'scan', event: 'repo_found', data: {} });
      fake.emit({ topic: 'scan', event: 'run_started', data: {} });
    });
    expect(fake.calls.filter((call) => call.name === 'projects.list')).toHaveLength(1);
  });

  it('a refused projects.list leaves the library uncomputed rather than empty', async () => {
    const fake = fakeAppDeps({
      'projects.list': () => {
        throw new Error('core is down');
      },
    });
    const view = mount(fake);
    await waitFor(() => {
      expect(fake.calls).toHaveLength(1);
    });
    // Still `null`. A failed read that rendered `[]` would say "no repositories" about a
    // library the app could not read — the sharpest form of unknown-as-zero.
    expect(view.last().rows).toBeNull();
  });
});
