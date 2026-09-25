/**
 * §1.4's card weighs each address by what the library holds, and that weight is written by the
 * authorship job — which runs after a walk has already ended. The card read the set at the end of
 * the walk and never again, so after a first run it went on saying `NO COMMITS IN THIS LIBRARY`
 * about an address with forty-five, until a relaunch. Driven through the real `App` and notice
 * slot against a fake core whose answer moves when the job settles.
 */
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import type {
  IdentityId,
  IdentityRow,
  ProjectId,
  ScanStatus,
  Settings,
  ViewState,
} from '../generated/protocol';
import type { RendererEvent } from '../shared/channels';
import { App } from './App';
import { AUTHORSHIP_REREAD_MS } from './app/useLibrary';
import { fakeAppDeps, type FakeAppDeps } from './app/testDeps';
import { makeProjectRow } from './testing/projectRow';

afterEach(cleanup);

const NOW = 1_700_000_000;

const scanned: ScanStatus = {
  runId: null,
  running: false,
  generation: 3,
  mode: null,
  startedAt: NOW - 100,
  endedAt: NOW - 50,
  cancelled: false,
  walkedDirs: 10,
  foundRepos: 4,
  indexedProjects: 4,
  problemCount: 0,
  ambiguousLineageCount: 0,
};

const view: ViewState = {
  query: '',
  sort: 'last_touched',
  viewMode: 'grid',
  density: 186,
  collapsedSections: [],
  scrollOffset: 0,
  selectedProjectId: null,
  dismissedNotices: [],
  windowGeometry: null,
  savedAt: NOW,
};

const settings: Settings = {
  effectsTier: 'off',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
  healthChecks: [],
};

function address(projects: number, commits: number): IdentityRow {
  return {
    id: 1 as IdentityId,
    email: 'e2e@example.invalid',
    name: null,
    isUser: true,
    source: 'gitconfig',
    aliasReason: null,
    primaryEmail: null,
    repositories: projects === 0 ? null : projects,
    commits,
    projects,
    confirmedAt: null,
  };
}

interface Mounted {
  readonly fake: FakeAppDeps;
  /** Moves what the core answers `identity.list` with from now on. */
  readonly settle: (row: IdentityRow) => void;
}

function mount(first: IdentityRow): Mounted {
  let current = first;
  const fake = fakeAppDeps(
    {
      'projects.list': () => ({
        sections: [],
        rows: [makeProjectRow({ id: 1 as ProjectId, name: 'alpha', lastTouchedAt: NOW - 3600 })],
        window: { from: 0, to: 1 },
        orderKey: 'k',
        generation: 3,
      }),
      'scan.status': () => scanned,
      'view.get': () => view,
      'view.set': () => ({}),
      'roots.suggest': () => [],
      'roots.list': () => [],
      'settings.get': () => settings,
      'targets.list': () => ({ resolved: null, rows: [] }),
      'identity.list': () => [current],
      'identity.confirm': () => ({ movedToReference: 0, commitDaysRemoved: 0, applied: false }),
      'problems.list': () => ({
        runId: null,
        header: { walkedDirs: 0, repositories: 0, problemCount: null, ambiguousLineageCount: null },
        groups: [],
      }),
    },
    { effectsTier: 'off' },
  );
  fake.setNow(NOW);
  render(<App deps={fake.deps} />);
  return {
    fake,
    settle: (row) => {
      current = row;
    },
  };
}

const jobDone = (job: string, projectId: number): RendererEvent => ({
  topic: 'scan',
  event: 'job_done',
  data: { projectId, locationId: projectId, job, state: 'ok' },
});

const reads = (fake: FakeAppDeps): number =>
  fake.calls.filter((call) => call.name === 'identity.list').length;

const sourceLine = async (): Promise<string> =>
  (await screen.findByText(/^GIT CONFIG ·/u)).textContent;

describe('§1.4 the identity card follows the authorship job', () => {
  it('re-reads the set when authorship settles after the walk ended', async () => {
    const { fake, settle } = mount(address(0, 0));
    expect(await sourceLine()).toBe('GIT CONFIG · GLOBAL · NO COMMITS IN THIS LIBRARY');

    settle(address(4, 6));
    act(() => {
      fake.emit(jobDone('j1_5', 4));
    });
    await waitFor(
      () => {
        expect(screen.getByText(/^GIT CONFIG ·/u).textContent).toBe(
          'GIT CONFIG · 4 REPOSITORIES · 6 COMMITS IN 4 PROJECTS',
        );
      },
      { timeout: AUTHORSHIP_REREAD_MS + 1000 },
    );
  });

  // A first scan settles authorship once per repository. One read per settle would be a read per
  // repository; the card needs the set as it stands once the burst has landed.
  it('a burst of settles costs one read, not one per repository', async () => {
    const { fake, settle } = mount(address(1, 1));
    await sourceLine();
    const before = reads(fake);

    settle(address(30, 45));
    act(() => {
      for (let id = 1; id <= 30; id += 1) fake.emit(jobDone('j1_5', id));
    });
    await waitFor(
      () => {
        expect(screen.getByText(/^GIT CONFIG ·/u).textContent).toBe(
          'GIT CONFIG · 30 REPOSITORIES · 45 COMMITS IN 30 PROJECTS',
        );
      },
      { timeout: AUTHORSHIP_REREAD_MS + 1000 },
    );
    expect(reads(fake) - before).toBeLessThanOrEqual(2);
  });

  it('a settle of any other job reads nothing', async () => {
    const { fake } = mount(address(1, 1));
    await sourceLine();
    const before = reads(fake);
    act(() => {
      fake.emit(jobDone('j2', 1));
      fake.emit(jobDone('j4', 1));
    });
    await new Promise((resolve) => setTimeout(resolve, AUTHORSHIP_REREAD_MS + 200));
    expect(reads(fake)).toBe(before);
  });

  // A run's last word re-reads the set at once, and that read is issued after every settle that
  // came before it — so a re-read still pending from one of those settles has nothing left to do.
  it('the read at the end of a run absorbs a re-read still pending', async () => {
    const { fake } = mount(address(1, 1));
    await sourceLine();
    const before = reads(fake);
    act(() => {
      fake.emit(jobDone('j1_5', 1));
      fake.emit({ topic: 'scan', event: 'finished', data: {} });
    });
    await new Promise((resolve) => setTimeout(resolve, AUTHORSHIP_REREAD_MS + 200));
    expect(reads(fake) - before).toBe(1);
  });
});
