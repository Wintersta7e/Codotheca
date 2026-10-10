import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it } from 'vitest';

import type { ProjectId, ProjectPage } from '../../src/generated/protocol';
import { fakeAppDeps, type FakeAppDeps } from '../../src/renderer/app/testDeps';
import { useLibrary } from '../../src/renderer/app/useLibrary';
import { useSync } from '../../src/renderer/app/useSync';
import { useInstall } from '../../src/renderer/install/useInstall';
import { createEventFanout } from '../../src/renderer/project/deps';
import { makeProjectRow } from '../../src/renderer/testing/projectRow';
import topicSnapshots from '../fixtures/topicSnapshots.json';

/**
 * The renderer half of every other topic's snapshot path. `src/main/core/bridge.test.ts` sends each
 * frame through the real client and bridge and asserts the batch it leaves as; that batch enters
 * here through the renderer's own fan-out, and the hook reading the topic must take it.
 */

interface Renderer extends FakeAppDeps {
  /** Hands every fan-out sink one batch, as the preload's listener would. */
  readonly deliver: (batch: unknown) => void;
}

function renderer(rows: { id: number; name: string }[] = []): Renderer {
  const sinks: ((batch: unknown) => void)[] = [];
  const page: ProjectPage = {
    sections: [],
    rows: rows.map(({ id, name }) => makeProjectRow({ id: id as ProjectId, name })),
    window: { from: 0, to: rows.length },
    orderKey: 'k',
    generation: 3,
  };
  const fake = fakeAppDeps(
    { 'projects.list': () => page },
    {
      subscribe: createEventFanout((cb) => {
        sinks.push(cb);
      }),
    },
  );
  return {
    ...fake,
    deliver: (batch) => {
      act(() => {
        for (const sink of sinks) sink(batch);
      });
    },
  };
}

it('the projects snapshot batch replaces the shelf without another read', async () => {
  const shell = renderer([{ id: 1, name: 'stale' }]);
  const { result } = renderHook(() => useLibrary(shell.deps));
  await waitFor(() => {
    expect(result.current.rows?.map((row) => row.name)).toEqual(['stale']);
  });

  shell.deliver(topicSnapshots.projects.batch);

  expect(result.current.rows?.map((row) => row.name)).toEqual(['alpha', 'beta']);
  expect(result.current.generation).toBe(7);
  expect(shell.calls.map((call) => call.name)).toEqual(['projects.list']);
});

it('the sync snapshot batch becomes the status, its listing and its notice', () => {
  const shell = renderer();
  const { result } = renderHook(() => useSync(shell.deps));
  expect(result.current.status).toBeNull();

  shell.deliver(topicSnapshots.sync.batch);

  expect(result.current.status).toEqual(topicSnapshots.sync.batch[0]?.data);
  expect(result.current.progressLine).toBe('41');
  expect(result.current.notice).toBe('throttled');
});

it('the install snapshot batch gives a tile mounted mid-clone its run', () => {
  const shell = renderer();
  const { result } = renderHook(() => useInstall(shell.deps, 5 as ProjectId));
  expect(result.current).toBeNull();

  shell.deliver(topicSnapshots.install.batch);

  expect(result.current).toEqual({
    runId: 9,
    observed: [{ runId: 9, stage: 'receiving', done: 120, total: null, bytes: 65536 }],
    destinationDisplay: '/w/gamma',
    failed: null,
    finished: null,
  });
});

// `beta` was announced only by the delta the gap lost, and `alpha`'s pin only by the frame that
// revealed the gap, which is never delivered: both reach the shelf through the snapshot alone.
it('the batch a projects gap ends in recovers the shelf without another read', async () => {
  const shell = renderer([{ id: 1, name: 'alpha' }]);
  const { result } = renderHook(() => useLibrary(shell.deps));
  await waitFor(() => {
    expect(result.current.rows?.map((row) => row.name)).toEqual(['alpha']);
  });

  shell.deliver(topicSnapshots.gap.batch);

  expect(result.current.rows?.map((row) => [row.name, row.isDirty, row.isPinned])).toEqual([
    ['alpha', true, true],
    ['beta', null, true],
  ]);
  expect(result.current.generation).toBe(4);
  expect(shell.calls.map((call) => call.name)).toEqual(['projects.list']);
});
