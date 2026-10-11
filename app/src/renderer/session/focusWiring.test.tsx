/**
 * §39.3a's wiring, through the real composition root: the reporter is built by the production
 * tree and fed by the project page, and a view that nobody touches claims nothing.
 */
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { App } from '../App';
import { fakeAppDeps, type FakeAppDeps } from '../app/testDeps';
import type {
  ProjectId,
  ProjectPage,
  ScanStatus,
  Settings,
  ViewState,
} from '../../generated/protocol';
import { PROJECT_PAGE_ROOT_CLASS } from '../project/ProjectPage';
import { detailFixture } from '../project/testFixtures';
import { makeProjectRow } from '../testing/projectRow';
import { required } from '../../shared/required';
import { FOCUS_HEARTBEAT_MS, type FocusReport } from '../../shared/sessionFocus';
import { createFocusReporter, type FocusReporter } from './focusReporter';

vi.mock('./focusReporter', async (importOriginal) => {
  const real: { createFocusReporter: typeof createFocusReporter } = await importOriginal();
  // The real reporter, observed: the test proves the tree built it, never builds one itself.
  return { ...real, createFocusReporter: vi.fn(real.createFocusReporter) };
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

const NOW = 1_700_000_000;
const PAGE_ID = 9 as ProjectId;

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

// §8.1's reference tail renders as text rows outside the virtualizer's window, so it is the row
// jsdom can click — the grid mounts no card in a zero-sized viewport.
const reference = makeProjectRow({
  id: PAGE_ID,
  name: 'borrowed',
  isReference: true,
  lastTouchedAt: NOW - 3600,
});

const shelf: ProjectPage = {
  sections: [],
  rows: [reference],
  window: { from: 0, to: 1 },
  orderKey: 'k',
  generation: 3,
};

function mount(): FakeAppDeps {
  const fake = fakeAppDeps(
    {
      'projects.list': () => shelf,
      'scan.status': () => scanned,
      'view.get': () => view,
      'view.set': () => ({}),
      'roots.suggest': () => [],
      'roots.list': () => [],
      'settings.get': () => settings,
      'targets.list': () => ({ resolved: null, rows: [] }),
      'identity.list': () => [],
      'projects.get': () => detailFixture({ row: reference }),
      'problems.list': () => ({
        runId: null,
        header: { walkedDirs: 0, repositories: 0, problemCount: null, ambiguousLineageCount: null },
        groups: [],
      }),
      'session.focus': () => ({}),
    },
    { effectsTier: 'off' },
  );
  fake.setNow(NOW);
  vi.mocked(createFocusReporter).mockClear();
  render(<App deps={fake.deps} />);
  return fake;
}

async function openPage(): Promise<HTMLElement> {
  await waitFor(() => {
    expect(document.querySelector('.cdt-reference-row')).not.toBeNull();
  });
  fireEvent.click(required(document.querySelector('.cdt-reference-row'), 'reference row'));
  await waitFor(() => {
    expect(document.querySelector(`.${PROJECT_PAGE_ROOT_CLASS}`)).not.toBeNull();
  });
  return required(document.querySelector<HTMLElement>(`.${PROJECT_PAGE_ROOT_CLASS}`), 'page');
}

function focusReports(fake: FakeAppDeps): (number | null)[] {
  return fake.calls
    .filter((call) => call.name === 'session.focus')
    .map((call) => (call.args as FocusReport).projectId);
}

/** The one reporter the tree built, so a test can stand in for input jsdom cannot make trusted. */
function builtReporter(): FocusReporter {
  const built = vi.mocked(createFocusReporter).mock.results;
  expect(built.length, 'the app built no focus reporter').toBe(1);
  const result = required(built[0], 'the built reporter');
  if (result.type !== 'return') throw new Error('createFocusReporter threw');
  return result.value;
}

describe('§39.3a the focus reporter, as the app mounts it', () => {
  it('a project page nobody touches claims nothing, and synthetic input claims nothing', async () => {
    const fake = mount();
    const root = await openPage();

    vi.useFakeTimers();
    act(() => {
      vi.advanceTimersByTime(5 * FOCUS_HEARTBEAT_MS);
    });
    // jsdom's dispatched events are untrusted, as a script's would be in the product.
    fireEvent.pointerDown(root);
    fireEvent.keyDown(root, { key: 'a', code: 'KeyA' });
    fireEvent.wheel(root);
    act(() => {
      vi.advanceTimersByTime(5 * FOCUS_HEARTBEAT_MS);
    });

    expect(focusReports(fake).filter((id) => id !== null)).toEqual([]);
    expect(builtReporter().claimed()).toBeNull();
  });

  it("the app's reporter reports through the core door, and leaving the page releases", async () => {
    const fake = mount();
    const root = await openPage();

    act(() => {
      builtReporter().input(PAGE_ID, true);
    });
    expect(focusReports(fake)).toEqual([PAGE_ID]);

    fireEvent.keyDown(root, { key: 'Escape', code: 'Escape' });
    await waitFor(() => {
      expect(document.querySelector(`.${PROJECT_PAGE_ROOT_CLASS}`)).toBeNull();
    });
    expect(focusReports(fake)).toEqual([PAGE_ID, null]);
  });
});
