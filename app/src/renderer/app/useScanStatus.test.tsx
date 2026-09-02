import { act, render, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';
import { describe, expect, it } from 'vitest';

import type { ScanRunId, ScanStatus } from '../../generated/protocol';
import type { AppDeps } from './deps';
import { fakeAppDeps, type FakeAppDeps } from './testDeps';
import { useScanStatus } from './useScanStatus';

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

function Probe({ deps, seen }: { deps: AppDeps; seen: (ScanStatus | null)[] }): ReactElement {
  seen.push(useScanStatus(deps));
  return <div />;
}

function mount(fake: FakeAppDeps): { last: () => ScanStatus | null } {
  const seen: (ScanStatus | null)[] = [];
  render(<Probe deps={fake.deps} seen={seen} />);
  return {
    last: () => {
      if (seen.length === 0) throw new Error('the hook rendered nothing');
      return seen[seen.length - 1] ?? null;
    },
  };
}

describe('useScanStatus', () => {
  it('is null until the core answers, which the gate reads as wait', async () => {
    const fake = fakeAppDeps({ 'scan.status': () => idle });
    const view = mount(fake);
    // `gateDecision(null, false)` is `'wait'`. A zeroed status here would answer `first-run`
    // or `shelf` before the core had said anything.
    expect(view.last()).toBeNull();
    await waitFor(() => {
      expect(view.last()).not.toBeNull();
    });
    expect(view.last()).toEqual(idle);
  });

  it('advances on the scan topic without asking the core again', async () => {
    const fake = fakeAppDeps({ 'scan.status': () => idle });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last()).not.toBeNull();
    });

    act(() => {
      fake.emit({
        topic: 'scan',
        event: 'run_started',
        data: {
          runId: 4,
          generation: 2,
          mode: 'full',
          roots: [],
          startedAt: 1_700_000_100,
        },
      });
    });
    expect(view.last()?.running).toBe(true);
    expect(view.last()?.runId).toBe(4 as ScanRunId);
    // A new run has counted no problems yet. Null is *not computed*; zero would be a claim.
    expect(view.last()?.problemCount).toBeNull();

    act(() => {
      fake.emit({
        topic: 'scan',
        event: 'progress',
        data: { runId: 4, walkedDirs: 900, foundRepos: 12, indexedProjects: 5 },
      });
    });
    expect(view.last()?.walkedDirs).toBe(900);
    expect(view.last()?.foundRepos).toBe(12);
    expect(view.last()?.indexedProjects).toBe(5);

    act(() => {
      fake.emit({
        topic: 'scan',
        event: 'finished',
        data: {
          runId: 4,
          endedAt: 1_700_000_400,
          walkedDirs: 1000,
          foundRepos: 14,
          problemCount: 2,
          ambiguousLineageCount: 0,
        },
      });
    });
    expect(view.last()?.running).toBe(false);
    expect(view.last()?.problemCount).toBe(2);
    expect(view.last()?.endedAt).toBe(1_700_000_400);
    expect(fake.calls.filter((c) => c.name === 'scan.status')).toHaveLength(1);
  });

  it('a cancelled run stops running and says it was cancelled', async () => {
    const fake = fakeAppDeps({
      'scan.status': () => ({ ...idle, running: true, runId: 4 as ScanRunId }),
    });
    const view = mount(fake);
    await waitFor(() => {
      expect(view.last()?.running).toBe(true);
    });
    act(() => {
      fake.emit({
        topic: 'scan',
        event: 'cancelled',
        data: { runId: 4, endedAt: 1_700_000_400, indexedProjects: 9 },
      });
    });
    expect(view.last()?.running).toBe(false);
    expect(view.last()?.cancelled).toBe(true);
    expect(view.last()?.indexedProjects).toBe(9);
  });

  it('an event before the core answers still leaves the status null', () => {
    const fake = fakeAppDeps({ 'scan.status': () => idle });
    const view = mount(fake);
    act(() => {
      fake.emit({
        topic: 'scan',
        event: 'progress',
        data: { runId: 4, walkedDirs: 1, foundRepos: 0, indexedProjects: 0 },
      });
    });
    // A partial status assembled from one progress frame would claim every other field.
    expect(view.last()).toBeNull();
  });
});
