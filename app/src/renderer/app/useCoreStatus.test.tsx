import { act, render } from '@testing-library/react';
import type { ReactElement } from 'react';
import { describe, expect, it } from 'vitest';

import type { AppDeps } from './deps';
import { fakeAppDeps, type FakeAppDeps } from './testDeps';
import { useCoreStatus, type CoreStatusState } from './useCoreStatus';

function Probe({ deps, seen }: { deps: AppDeps; seen: CoreStatusState[] }): ReactElement {
  seen.push(useCoreStatus(deps));
  return <div />;
}

function mount(fake: FakeAppDeps): { last: () => CoreStatusState; renders: () => number } {
  const seen: CoreStatusState[] = [];
  render(<Probe deps={fake.deps} seen={seen} />);
  return {
    renders: () => seen.length,
    last: () => {
      const state = seen.at(-1);
      if (state === undefined) throw new Error('the hook rendered nothing');
      return state;
    },
  };
}

describe('useCoreStatus', () => {
  it('starts at starting — never at ready, and never at failed', () => {
    const view = mount(fakeAppDeps());
    expect(view.last().lane.kind).toBe('starting');
    expect(view.last().startupFailure).toBeNull();
  });

  it('carries the log path the shell passed, for the failure windows to name', () => {
    const fake = fakeAppDeps({}, { logPath: '/var/log/codotheca.log' });
    expect(mount(fake).last().logPath).toBe('/var/log/codotheca.log');
  });

  it('takes the startup failure off the failed status, where it is current', () => {
    const fake = fakeAppDeps();
    const view = mount(fake);
    act(() => {
      fake.setCoreStatus({
        kind: 'failed',
        reason: 'crash_loop',
        detail: 'exit 4',
        logPath: '/var/log/codotheca.log',
        startupFailure: { kind: 'schema_from_future', onDisk: 9, supported: 5 },
      });
    });
    expect(view.last().lane.kind).toBe('failed');
    expect(view.last().startupFailure).toEqual({
      kind: 'schema_from_future',
      onDisk: 9,
      supported: 5,
    });
  });

  it('a failed lane with no report leaves the fact null rather than inventing one', () => {
    const fake = fakeAppDeps();
    const view = mount(fake);
    act(() => {
      fake.setCoreStatus({
        kind: 'failed',
        reason: 'spawn',
        detail: 'ENOENT',
        logPath: '/var/log/codotheca.log',
      });
    });
    expect(view.last().lane.kind).toBe('failed');
    // §11.2's five spawn sentences are the main process's; this hook does not author one.
    expect(view.last().startupFailure).toBeNull();
  });

  it('registers on the status channel once, however often it re-renders', () => {
    let registrations = 0;
    const fake = fakeAppDeps();
    const counting: AppDeps = {
      ...fake.deps,
      onCoreStatus: (cb) => {
        registrations += 1;
        return fake.deps.onCoreStatus(cb);
      },
    };
    const seen: CoreStatusState[] = [];
    render(<Probe deps={counting} seen={seen} />);
    act(() => {
      fake.setCoreStatus({ kind: 'restarting', epoch: 2, delayMs: 2000 });
    });
    act(() => {
      fake.setCoreStatus({ kind: 'restarting', epoch: 3, delayMs: 2000 });
    });
    // `onCoreStatus` has no disposer of its own; a hook that re-registered per render would
    // accumulate listeners silently for the window's whole life.
    expect(registrations).toBe(1);
  });

  it('reads core/degraded off the topic and clears it on the next snapshot', () => {
    const fake = fakeAppDeps();
    const view = mount(fake);
    expect(view.last().degraded).toBeNull();

    act(() => {
      fake.emit({
        topic: 'core',
        event: 'degraded',
        data: { reason: 'git_missing', detail: null },
      });
    });
    expect(view.last().degraded).toBe('git_missing');
  });

  it('takes first_run_completed_at from the core snapshot, which is where it is stated', () => {
    const fake = fakeAppDeps();
    const view = mount(fake);
    // Absent means first run has not finished, under which §10.5a says nothing is new. It is
    // never 0 — that would file every project as an arrival.
    expect(view.last().firstRunCompletedAt).toBeNull();

    act(() => {
      fake.emit({
        topic: 'core',
        event: 'snapshot',
        data: {
          epoch: 1,
          throughSeq: 4,
          gitVersion: '2.44.0',
          schemaVersion: 5,
          scan: null,
          firstRunCompletedAt: 1_700_000_500,
        },
      });
    });
    expect(view.last().firstRunCompletedAt).toBe(1_700_000_500);
  });
});
