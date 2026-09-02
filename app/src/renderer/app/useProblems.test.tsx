import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { Problems, ScanRunId, ScanStatus } from '../../generated/protocol';
import { fakeAppDeps } from './testDeps';
import { useProblems } from './useProblems';

const idle: ScanStatus = {
  runId: null,
  running: false,
  generation: null,
  mode: null,
  startedAt: null,
  endedAt: null,
  cancelled: false,
  walkedDirs: 0,
  foundRepos: 0,
  indexedProjects: 0,
  problemCount: null,
  ambiguousLineageCount: null,
};

function problems(runId: number): Problems {
  return {
    runId: runId as ScanRunId,
    header: { walkedDirs: 1, repositories: 1, problemCount: 2, ambiguousLineageCount: 0 },
    groups: [],
  };
}

describe('useProblems', () => {
  it('asks for nothing before a run has finished', () => {
    const fake = fakeAppDeps({ 'problems.list': () => problems(1) });
    renderHook(() => useProblems(fake.deps, { ...idle, running: true, runId: 1 as ScanRunId }));
    // A run in flight has no final count; asking mid-walk would report a figure that changes.
    expect(fake.calls.filter((c) => c.name === 'problems.list')).toHaveLength(0);
  });

  it('is null until the core answers, never an empty report', async () => {
    const fake = fakeAppDeps({ 'problems.list': () => problems(4) });
    const hook = renderHook(() =>
      useProblems(fake.deps, { ...idle, runId: 4 as ScanRunId, endedAt: 1 }),
    );
    expect(hook.result.current.problems).toBeNull();
    await waitFor(() => {
      expect(hook.result.current.problems).not.toBeNull();
    });
    expect(hook.result.current.problems?.runId).toBe(4);
  });

  it('asks again when a new run finishes, and not otherwise', async () => {
    const fake = fakeAppDeps({ 'problems.list': (args) => problems(Number(args.run ?? 0)) });
    const finished = { ...idle, runId: 4 as ScanRunId, endedAt: 1 };
    const hook = renderHook(({ scan }: { scan: ScanStatus }) => useProblems(fake.deps, scan), {
      initialProps: { scan: finished },
    });
    await waitFor(() => {
      expect(hook.result.current.problems?.runId).toBe(4);
    });

    hook.rerender({ scan: { ...finished, walkedDirs: 99 } });
    expect(fake.calls.filter((c) => c.name === 'problems.list')).toHaveLength(1);

    hook.rerender({ scan: { ...finished, runId: 5 as ScanRunId } });
    await waitFor(() => {
      expect(hook.result.current.problems?.runId).toBe(5);
    });
    expect(fake.calls.filter((c) => c.name === 'problems.list')).toHaveLength(2);
  });

  it('reload asks again for the same run, which is what a repair needs', async () => {
    const fake = fakeAppDeps({ 'problems.list': () => problems(4) });
    const hook = renderHook(() =>
      useProblems(fake.deps, { ...idle, runId: 4 as ScanRunId, endedAt: 1 }),
    );
    await waitFor(() => {
      expect(hook.result.current.problems).not.toBeNull();
    });
    act(() => {
      hook.result.current.reload();
    });
    await waitFor(() => {
      expect(fake.calls.filter((c) => c.name === 'problems.list')).toHaveLength(2);
    });
  });

  it('a refused read leaves the report uncomputed rather than empty', async () => {
    const fake = fakeAppDeps({
      'problems.list': () => {
        throw new Error('core is down');
      },
    });
    const hook = renderHook(() =>
      useProblems(fake.deps, { ...idle, runId: 4 as ScanRunId, endedAt: 1 }),
    );
    await waitFor(() => {
      expect(fake.calls.filter((c) => c.name === 'problems.list')).toHaveLength(1);
    });
    expect(hook.result.current.problems).toBeNull();
  });
});
