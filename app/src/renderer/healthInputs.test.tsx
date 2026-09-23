/**
 * [p3] §30.9 — a check switched off in the drawer takes its items off what is already on screen,
 * and the source-reading grant given from the `HEALTH` tab re-reads the same way.
 *
 * `settings.set` raises no event, so the composition root carries the write to the readings it
 * holds: the shelf's rows and an open page's detail. Driven through the real `App`, drawer and
 * page, against a fake core that answers `projects.get` the way the real one does — without the
 * items of a check that is switched off, and with `todo_marker` `off` while the grant is missing.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import type {
  DebtItem,
  DebtSource,
  HealthCheck,
  HealthReading,
  ProjectId,
  ScanStatus,
  Settings,
  SettingsPatch,
  ViewState,
} from '../generated/protocol';
import { App } from './App';
import { fakeAppDeps, type FakeAppDeps } from './app/testDeps';
import { GRANT_ASK_ACTION } from './project/health/GrantAsk';
import { SOURCE_LABELS } from './project/health/labels';
import { detailFixture, rowFixture } from './project/testFixtures';
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
  foundRepos: 1,
  indexedProjects: 1,
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

function item(over: Partial<DebtItem>): DebtItem {
  return {
    source: 'missing_readme',
    fingerprint: '',
    state: 'open',
    scoring: 'scored',
    layer: 'dust',
    pathDisplay: null,
    line: null,
    column: null,
    salientText: null,
    firstSeenAt: NOW - 3600,
    lastSeenAt: NOW - 60,
    basis: 'head',
    advisory: null,
    ...over,
  };
}

const README = item({ source: 'missing_readme' });
const MARKER = item({
  source: 'todo_marker',
  fingerprint: 'f00d:0',
  layer: 'overgrowth',
  pathDisplay: 'src/lib.rs',
  line: 3,
  salientText: 'TODO: a shaped marker',
});

/**
 * What the core derives from the switches and the grant. A check switched off, or `todo_marker`
 * without the grant, is `off` with its items gone (§30.9, R128/F8). A grant given since the last
 * sweep has not run yet, so the check is `unknown` and has found nothing.
 */
function detailFor(settings: Settings, sweptWithGrant: boolean): ReturnType<typeof detailFixture> {
  const on = (check: DebtSource): boolean =>
    settings.healthChecks.find((entry) => entry.check === check)?.enabled ?? true;
  const readme: HealthCheck = {
    id: 'missing_readme',
    outcome: on('missing_readme') ? 'failed' : 'off',
    unknownReason: null,
  };
  const marker: HealthCheck =
    !on('todo_marker') || !settings.contentScanEnabled
      ? { id: 'todo_marker', outcome: 'off', unknownReason: null }
      : sweptWithGrant
        ? { id: 'todo_marker', outcome: 'failed', unknownReason: null }
        : { id: 'todo_marker', outcome: 'unknown', unknownReason: 'notRunYet' };
  const checks = [readme, marker];
  const debt = [
    ...(readme.outcome === 'failed' ? [README] : []),
    ...(marker.outcome === 'failed' ? [MARKER] : []),
  ];
  const ran = checks.filter((c) => c.outcome === 'failed').length;
  const unknown = checks.filter((c) => c.outcome === 'unknown').length;
  const health: HealthReading = {
    state: 'live',
    scoredOpen: debt.length,
    basis: {
      ran,
      eligible: ran + unknown,
      unknown,
      off: checks.filter((c) => c.outcome === 'off').length,
      notApplicable: 0,
      observedAt: NOW - 30,
    },
    checks,
  };
  return detailFixture({ row: rowFixture({ id: 9 as ProjectId }), health, debt });
}

/** Applies a patch as the core does: only the fields and the check entries it names. */
function applied(current: Settings, patch: SettingsPatch): Settings {
  return {
    ...current,
    autostart: patch.autostart ?? current.autostart,
    contentScanEnabled: patch.contentScanEnabled ?? current.contentScanEnabled,
    healthChecks: current.healthChecks.map(
      (entry) => patch.healthChecks?.find((p) => p.check === entry.check) ?? entry,
    ),
  };
}

function mount(contentScanEnabled = true): FakeAppDeps {
  let settings: Settings = {
    effectsTier: 'off',
    reducedMotionOverride: false,
    autostart: false,
    residentShortcut: null,
    roastEnabled: true,
    logLevel: 'info',
    installRootId: null,
    contentScanEnabled,
    healthChecks: [
      { check: 'todo_marker', enabled: true },
      { check: 'missing_readme', enabled: true },
    ],
  };
  // The shelf's reference tail is the one row jsdom mounts — the grid's virtualizer has a
  // zero-sized viewport — so it is the row clicked. The page draws its own `projects.get`.
  const shelfRow = makeProjectRow({
    id: 9 as ProjectId,
    name: 'borrowed',
    isReference: true,
    lastTouchedAt: NOW - 3600,
  });
  const fake = fakeAppDeps(
    {
      'projects.list': () => ({
        sections: [],
        rows: [shelfRow],
        window: { from: 0, to: 1 },
        orderKey: 'k',
        generation: 3,
      }),
      'scan.status': () => scanned,
      'view.get': () => view,
      'view.set': () => ({}),
      'roots.suggest': () => [],
      'roots.list': () => [],
      'targets.list': () => ({ resolved: null, rows: [] }),
      'identity.list': () => [],
      // The drawer mounts its account panel, which reads this on open.
      'accounts.list': () => [],
      'problems.list': () => ({
        runId: null,
        header: { walkedDirs: 0, repositories: 0, problemCount: null, ambiguousLineageCount: null },
        groups: [],
      }),
      'settings.get': () => settings,
      'settings.set': ({ patch }) => {
        settings = applied(settings, patch);
        return settings;
      },
      'projects.get': () => detailFor(settings, contentScanEnabled),
    },
    { effectsTier: 'off' },
  );
  fake.setNow(NOW);
  render(<App deps={fake.deps} />);
  return fake;
}

const count = (fake: FakeAppDeps, name: string): number =>
  fake.calls.filter((call) => call.name === name).length;

async function openHealthTab(): Promise<void> {
  await waitFor(() => {
    expect(document.querySelector('.cdt-reference-row')).not.toBeNull();
  });
  fireEvent.click(document.querySelector('.cdt-reference-row') as HTMLElement);
  fireEvent.click(await screen.findByRole('tab', { name: 'HEALTH' }));
}

/**
 * The drawer opens from the shelf's top bar, or from a page's install upgrade — and that one is
 * offered only where there is no working copy, which carries no `HEALTH` tab. So the page is
 * opened behind a drawer already open, which is the state the wiring has to survive either way.
 */
async function pageBehindDrawer(fake: FakeAppDeps): Promise<HTMLElement> {
  await waitFor(() => {
    expect(document.querySelector('.cdt-reference-row')).not.toBeNull();
  });
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  await screen.findByRole('switch', { name: SOURCE_LABELS.missing_readme });
  await openHealthTab();
  const list = await screen.findByTestId('cp-health-debt');
  expect(list.textContent).toContain(SOURCE_LABELS.missing_readme);
  expect(count(fake, 'projects.get')).toBe(1);
  return list;
}

/** The `HEALTH` tab's row for one check, found by its raw id. */
function checkRow(id: DebtSource): HTMLElement {
  const row = document.querySelector<HTMLElement>(`.cp-health-check[data-check="${id}"]`);
  if (row === null) throw new Error(`the HEALTH tab draws no ${id} row`);
  return row;
}

describe('§30.9 a switch reaches the readings already on screen', () => {
  it('switching a check off re-reads the open page and the shelf, and its items leave the list', async () => {
    const fake = mount();
    await pageBehindDrawer(fake);
    const listsBefore = count(fake, 'projects.list');

    fireEvent.click(screen.getByRole('switch', { name: SOURCE_LABELS.missing_readme }));
    await waitFor(() => {
      expect(count(fake, 'projects.get')).toBe(2);
    });
    await waitFor(() => {
      expect(screen.getByTestId('cp-health-debt').textContent).not.toContain(
        SOURCE_LABELS.missing_readme,
      );
    });
    // The list re-rendered from the second answer rather than emptying: the other check's item
    // is still there.
    expect(screen.getByTestId('cp-health-debt').textContent).toContain(SOURCE_LABELS.todo_marker);
    expect(count(fake, 'projects.list')).toBe(listsBefore + 1);
  });

  it('a write that moves no reading re-reads nothing', async () => {
    const fake = mount();
    await pageBehindDrawer(fake);
    const listsBefore = count(fake, 'projects.list');

    const autostart = screen.getByRole('switch', { name: 'Start with the system' });
    fireEvent.click(autostart);
    // The drawer takes the core's answer, so the switch moving is the write having landed.
    await waitFor(() => {
      expect(autostart.getAttribute('aria-checked')).toBe('true');
    });
    expect(count(fake, 'projects.get')).toBe(1);
    expect(count(fake, 'projects.list')).toBe(listsBefore);
  });

  it('R142 granting source reading from the tab re-reads the page and the shelf, and todo_marker stops reading switched off', async () => {
    const fake = mount(false);
    await openHealthTab();
    await screen.findByTestId('cp-health-ask');
    expect(checkRow('todo_marker').getAttribute('data-outcome')).toBe('off');
    expect(count(fake, 'projects.get')).toBe(1);
    const listsBefore = count(fake, 'projects.list');

    fireEvent.click(screen.getByRole('button', { name: GRANT_ASK_ACTION }));
    await waitFor(() => {
      expect(count(fake, 'projects.get')).toBe(2);
    });
    await waitFor(() => {
      expect(checkRow('todo_marker').getAttribute('data-outcome')).toBe('unknown');
    });
    // Granted and not yet swept is `unknown`, named — never the user's own switch.
    expect(checkRow('todo_marker').textContent).not.toMatch(/switched this check off/u);
    expect(screen.queryByTestId('cp-health-ask')).toBeNull();
    expect(count(fake, 'projects.list')).toBe(listsBefore + 1);
  });
});
