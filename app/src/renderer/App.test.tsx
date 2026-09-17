import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { App } from './App';
import { EFFECTS_TIER_ATTRIBUTE } from './effectsTier';
import { fakeAppDeps, type FakeAppDeps, type FakeReplies } from './app/testDeps';
import type {
  IdentityId,
  ProjectId,
  ProjectPage,
  ProjectRow,
  ScanStatus,
  Settings,
  ViewState,
} from '../generated/protocol';
import { detailFixture } from './project/testFixtures';
import { PROJECT_PAGE_ROOT_CLASS } from './project/ProjectPage';
import { makeProjectRow } from './testing/projectRow';

afterEach(() => {
  cleanup();
});

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
  foundRepos: 2,
  indexedProjects: 2,
  problemCount: 0,
  ambiguousLineageCount: 0,
};

const neverScanned: ScanStatus = {
  ...scanned,
  generation: null,
  startedAt: null,
  endedAt: null,
  foundRepos: 0,
  indexedProjects: 0,
  problemCount: null,
  ambiguousLineageCount: null,
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
};

function page(rows: readonly ProjectRow[]): ProjectPage {
  return {
    sections: [],
    rows: [...rows],
    window: { from: 0, to: rows.length },
    orderKey: 'k',
    generation: 3,
  };
}

function repliesFor(rows: readonly ProjectRow[], status: ScanStatus): FakeReplies {
  return {
    'projects.list': () => page(rows),
    'scan.status': () => status,
    'view.get': () => view,
    'view.set': () => ({}),
    'roots.suggest': () => [],
    'roots.list': () => [],
    'settings.get': () => settings,
    'targets.list': () => ({ resolved: null, rows: [] }),
    'identity.list': () => [],
    'projects.get': (args) => {
      const found = rows.find((r) => r.id === args.id);
      return found === undefined ? detailFixture() : detailFixture({ row: found });
    },
    'problems.list': () => ({
      runId: null,
      header: { walkedDirs: 0, repositories: 0, problemCount: null, ambiguousLineageCount: null },
      groups: [],
    }),
  };
}

function mount(rows: readonly ProjectRow[], status: ScanStatus): FakeAppDeps {
  const fake = fakeAppDeps(repliesFor(rows, status), { effectsTier: 'off' });
  fake.setNow(NOW);
  render(<App deps={fake.deps} />);
  return fake;
}

const row = (id: number, name: string): ProjectRow =>
  makeProjectRow({ id: id as ProjectId, name, lastTouchedAt: NOW - 3600 });

describe('App — the composition root', () => {
  it('paints the shelf for a scanned library', async () => {
    mount([row(1, 'alpha'), row(2, 'beta')], scanned);
    await waitFor(() => {
      expect(document.querySelector('.cdt-shelf')).not.toBeNull();
    });
    // The top bar and the one scroll container, from the real components rather than a stub.
    expect(document.querySelector('.cdt-shelf-scroll')).not.toBeNull();
  });

  // The resolver ran and its answer never reached the DOM. `main.tsx` writes the boot value once
  // before mount, `auto` is the default (`shared/bootFile.ts:49`), and nothing resolved it
  // afterwards — so every `[data-effects-tier='full'|'reduced'|'off']` rule in the repository
  // selected nothing, the project page's entrance and cascade among them. Asserted at the
  // composition root rather than on the hook, because the defect was a missing CALL: a hook-only
  // test passes while nobody calls it, which is the shape this project keeps getting caught by.
  it('resolves auto onto the document element, so no CSS tier rule is inert', async () => {
    const fake = fakeAppDeps(repliesFor([row(1, 'alpha')], scanned), { effectsTier: 'auto' });
    fake.setNow(NOW);
    render(<App deps={fake.deps} />);
    await waitFor(() => {
      expect(document.documentElement.getAttribute(EFFECTS_TIER_ATTRIBUTE)).toBe('full');
    });
  });

  /**
   * §11.3a forbids a control that promises something and does nothing, and this is the one the
   * user pressed. Every unit below it is covered — `NoticeSlot` dismisses by the scoped key,
   * `selectNotice` filters on that key, `useViewState` sets local state before it writes — but
   * nothing had ever asserted the chain end to end with a *real* scan notice, because the App
   * fixture answers `problems.list` with `runId: null`, which produces no banner at all.
   */
  it('dismisses a scan problems notice, and it stays dismissed', async () => {
    // A run id on both sides: `useProblems` asks for nothing without one, and `problemsNotice`
    // returns null without one, so the default fixture's `runId: null` raises no banner at all.
    const withRun: ScanStatus = { ...scanned, runId: 7 as ScanStatus['runId'], problemCount: 2 };
    const fake = fakeAppDeps(
      {
        ...repliesFor([row(1, 'alpha')], withRun),
        'problems.list': () => ({
          runId: withRun.runId,
          header: {
            walkedDirs: 12,
            repositories: 3,
            problemCount: 2,
            ambiguousLineageCount: null,
          },
          groups: [],
        }),
      },
      { effectsTier: 'off' },
    );
    fake.setNow(NOW);
    render(<App deps={fake.deps} />);

    const banner = await screen.findByText(/THE LAST SCAN LEFT 2 PROBLEMS/u);
    expect(banner).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: /dismiss/i }));

    await waitFor(() => {
      expect(screen.queryByText(/THE LAST SCAN LEFT 2 PROBLEMS/u)).toBeNull();
    });
    // And it does not come back on the next paint: a banner that returns is the same dead control
    // wearing a delay.
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText(/THE LAST SCAN LEFT 2 PROBLEMS/u)).toBeNull();
  });

  it('runs first run for a library that has never been scanned', async () => {
    mount([], neverScanned);
    await waitFor(() => {
      expect(document.querySelector('.cdt-fr-view')).not.toBeNull();
    });
    expect(document.querySelector('.cdt-shelf')).toBeNull();
  });

  it('never says setup, on any surface', async () => {
    mount([], neverScanned);
    await waitFor(() => {
      expect(document.querySelector('.cdt-fr-view')).not.toBeNull();
    });
    // §10.1: the word appears nowhere in first run. Asserted here in jsdom and again against a
    // real render in `app/e2e/mount.spec.ts`.
    expect((document.body.textContent ?? '').toLowerCase()).not.toContain('setup');
  });

  it('takes the whole screen for a fatal index, over everything behind it', async () => {
    const fake = mount([row(1, 'alpha')], scanned);
    await waitFor(() => {
      expect(document.querySelector('.cdt-shelf')).not.toBeNull();
    });

    act(() => {
      fake.setCoreStatus({
        kind: 'failed',
        reason: 'crash_loop',
        detail: 'exit 4',
        logPath: '/tmp/codotheca.log',
        startupFailure: { kind: 'schema_from_future', onDisk: 9, supported: 5 },
      });
    });
    expect(screen.getByText('THIS LIBRARY WAS WRITTEN BY A NEWER CODOTHECA')).toBeTruthy();
    // §11.2a is full screen because there is nothing behind it worth showing.
    expect(document.querySelector('.cdt-shelf')).toBeNull();
  });

  it('opens the project page and takes the shelf off the screen', async () => {
    // The grid mounts no card in jsdom — the virtualizer's viewport is zero-sized, so its
    // window is empty, which is correct. §8.1's reference tail is text rows outside that
    // window, so it is the row this environment can actually click.
    const reference = makeProjectRow({
      id: 9 as ProjectId,
      name: 'borrowed',
      isReference: true,
      lastTouchedAt: NOW - 3600,
    });
    mount([row(1, 'alpha'), reference], scanned);
    await waitFor(() => {
      expect(document.querySelector('.cdt-reference-row')).not.toBeNull();
    });

    fireEvent.click(document.querySelector('.cdt-reference-row') as HTMLElement);
    await waitFor(() => {
      expect(document.querySelector(`.${PROJECT_PAGE_ROOT_CLASS}`)).not.toBeNull();
    });
    expect(document.querySelector('.cdt-shelf')).toBeNull();
  });

  it('reads each of its channels exactly once at startup', async () => {
    const fake = mount([row(1, 'alpha')], scanned);
    await waitFor(() => {
      expect(document.querySelector('.cdt-shelf')).not.toBeNull();
    });
    for (const command of ['projects.list', 'scan.status', 'view.get'] as const) {
      expect(fake.calls.filter((call) => call.name === command)).toHaveLength(1);
    }
  });

  it('draws §1.4 card in §8.0 slot when the seeded set has not been confirmed', async () => {
    // `IdentityCard` was mounted nowhere: the component, its copy, its priority row and the
    // slot's own `renderContent` seam all existed, and nothing supplied it — so
    // `identity.confirm` was never called on any machine. The bar is the card **on screen**,
    // not the hook that would raise it.
    const fake = fakeAppDeps(
      {
        ...repliesFor([row(1, 'alpha')], scanned),
        'identity.list': () => [
          {
            id: 1 as IdentityId,
            isUser: true,
            email: 'a@example.invalid',
            name: null,
            source: 'gitconfig' as const,
            confirmedAt: null,
            commits: 12,
            projects: 1,
            repositories: null,
            primaryEmail: null,
            aliasReason: null,
          },
        ],
        'identity.confirm': () => ({
          movedToReference: 0,
          commitDaysRemoved: 0,
          applied: true,
        }),
      },
      { effectsTier: 'off' },
    );
    fake.setNow(NOW);
    render(<App deps={fake.deps} />);

    const card = await screen.findByText('WHAT COUNTS AS YOURS');
    expect(card).not.toBeNull();
    const tick = await screen.findByLabelText('a@example.invalid');
    expect(tick.getAttribute('type')).toBe('checkbox');

    fireEvent.click(screen.getByRole('button', { name: 'CONFIRM' }));
    await waitFor(() => {
      expect(
        fake.calls.filter(
          (call) =>
            call.name === 'identity.confirm' && (call.args as { apply?: boolean }).apply === true,
        ),
      ).toHaveLength(1);
    });
    // Authorship just moved, so the shelf it is computed into is re-read rather than left as it
    // was until the next launch.
    await waitFor(() => {
      expect(fake.calls.filter((call) => call.name === 'projects.list').length).toBeGreaterThan(1);
    });
  });

  it('raises no identity card over a set that has already been confirmed', async () => {
    const fake = fakeAppDeps(
      {
        ...repliesFor([row(1, 'alpha')], scanned),
        'identity.list': () => [
          {
            id: 1 as IdentityId,
            isUser: true,
            email: 'a@example.invalid',
            name: null,
            source: 'gitconfig' as const,
            confirmedAt: NOW - 100,
            commits: 12,
            projects: 1,
            repositories: null,
            primaryEmail: null,
            aliasReason: null,
          },
        ],
      },
      { effectsTier: 'off' },
    );
    fake.setNow(NOW);
    render(<App deps={fake.deps} />);
    await waitFor(() => {
      expect(document.querySelector('.cdt-shelf')).not.toBeNull();
    });
    expect(screen.queryByText('WHAT COUNTS AS YOURS')).toBeNull();
  });

  it('writes no view.set for a shelf nobody touched', async () => {
    const fake = mount([row(1, 'alpha')], scanned);
    await waitFor(() => {
      expect(document.querySelector('.cdt-shelf')).not.toBeNull();
    });
    expect(fake.calls.filter((call) => call.name === 'view.set')).toHaveLength(0);
  });
});
