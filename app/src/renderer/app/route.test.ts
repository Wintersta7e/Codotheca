import { describe, expect, it } from 'vitest';

import type { ProjectId, ScanStatus } from '../../generated/protocol';
import type { CoreStatus } from '../../shared/coreStatus';
import type { StartupFailure } from '../../shared/startupFailure';
import { routeFor, type RouteInput } from './route';

const scanned: ScanStatus = {
  runId: null,
  running: false,
  generation: 3,
  mode: null,
  startedAt: 1_700_000_000,
  endedAt: 1_700_000_100,
  cancelled: false,
  walkedDirs: 10,
  foundRepos: 2,
  indexedProjects: 2,
  problemCount: 0,
  ambiguousLineageCount: 0,
};

const neverScanned: ScanStatus = { ...scanned, generation: null, startedAt: null, endedAt: null };

const failure: StartupFailure = { kind: 'schema_from_future', onDisk: 9, supported: 5 };

const ready: CoreStatus = {
  kind: 'ready',
  epoch: 1,
  coreVersion: '0.0.0',
  protocolVersion: 2,
  pid: 10,
};
const failed: CoreStatus = {
  kind: 'failed',
  reason: 'crash_loop',
  detail: 'exit 4',
  logPath: '/tmp/x.log',
  startupFailure: failure,
};
const failedWithoutReport: CoreStatus = { ...failed, startupFailure: null };

function input(over: Partial<RouteInput> = {}): RouteInput {
  return {
    lane: ready,
    startupFailure: null,
    scan: scanned,
    hasStoredShelf: true,
    openProjectId: null,
    ...over,
  };
}

describe('routeFor', () => {
  it('is the shelf by default', () => {
    expect(routeFor(input())).toEqual({ kind: 'shelf' });
  });

  it('opens a project page, carrying the id in the route and not beside it', () => {
    expect(routeFor(input({ openProjectId: 12 as ProjectId }))).toEqual({
      kind: 'project',
      id: 12,
    });
  });

  it('waits for the core before deciding first run', () => {
    // `gateDecision(null, false)` is `'wait'`, and the gate owns the blank ground. Falling
    // through to the shelf here would flash an empty grid under the roots screen.
    expect(routeFor(input({ scan: null, hasStoredShelf: false }))).toEqual({ kind: 'first-run' });
  });

  it('runs first run for a library that has never been scanned', () => {
    expect(routeFor(input({ scan: neverScanned, hasStoredShelf: false }))).toEqual({
      kind: 'first-run',
    });
  });

  it('a stored shelf is proof first run is over', () => {
    expect(routeFor(input({ scan: neverScanned, hasStoredShelf: true }))).toEqual({
      kind: 'shelf',
    });
  });

  it('a failure outranks first run', () => {
    const route = routeFor(
      input({ lane: failed, startupFailure: failure, scan: null, hasStoredShelf: false }),
    );
    expect(route).toEqual({ kind: 'failure', fact: failure });
  });

  it('an open project page does not survive a failure arriving under it', () => {
    // §11.2a is full screen because there is nothing behind it to show: the index did not open,
    // so the page beneath is describing rows the core cannot read.
    const route = routeFor(
      input({ lane: failed, startupFailure: failure, openProjectId: 12 as ProjectId }),
    );
    expect(route).toEqual({ kind: 'failure', fact: failure });
  });

  it('a failed lane with no report is not a full-screen window', () => {
    // §11.2's five spawn sentences belong to the main process and are not written yet, and a
    // window with no copy is worse than the notice slot, which §8.0 gives priority 1.
    expect(routeFor(input({ lane: failedWithoutReport }))).toEqual({ kind: 'shelf' });
  });

  it('a report from a lane that recovered draws nothing', () => {
    // The report on disk is yesterday's if the core opened the index today. Only a lane that is
    // currently failed makes it current.
    expect(routeFor(input({ lane: ready, startupFailure: failure }))).toEqual({ kind: 'shelf' });
  });

  it('a restarting lane is not a failure', () => {
    expect(routeFor(input({ lane: { kind: 'restarting', epoch: 2, delayMs: 2000 } }))).toEqual({
      kind: 'shelf',
    });
  });
});
