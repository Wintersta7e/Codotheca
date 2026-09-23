/**
 * §11.3a's two motion rows reach §11.6's resolved tier on the document element, live.
 *
 * The drawer stored both and the window never heard: the root read only the tier the shell put on
 * the window's argv at launch, and passed the reduced-motion override as a literal `false`. A user
 * who picked `REDUCED` or `OFF` got full motion until something rewrote the boot file, which
 * nothing did. Driven through the real `App` and drawer against a fake core that stores what it is
 * sent, because the defect was a missing wire between two components that each worked alone.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import type { ProjectId, ScanStatus, Settings, ViewState } from '../generated/protocol';
import type { EffectsTier, EffectsTierSource } from '../shared/effectsTier';
import { App } from './App';
import { fakeAppDeps, type FakeAppDeps } from './app/testDeps';
import { EFFECTS_TIER_ATTRIBUTE } from './effectsTier';
import { makeProjectRow } from './testing/projectRow';

afterEach(cleanup);

const NOW = 1_700_000_000;
const OVERRIDE = 'Respect the system reduced-motion setting';

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

interface Launch {
  /** What the shell put on the window's argv, and where it came from. */
  readonly boot: EffectsTier;
  readonly source?: EffectsTierSource;
  /** What the core has stored. */
  readonly stored: EffectsTier;
  readonly override?: boolean;
}

function mount(launch: Launch): FakeAppDeps {
  let settings: Settings = {
    effectsTier: launch.stored,
    reducedMotionOverride: launch.override ?? false,
    autostart: false,
    residentShortcut: null,
    roastEnabled: true,
    logLevel: 'info',
    installRootId: null,
    contentScanEnabled: false,
    healthChecks: [],
  };
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
      'targets.list': () => ({ resolved: null, rows: [] }),
      'identity.list': () => [],
      'accounts.list': () => [],
      'problems.list': () => ({
        runId: null,
        header: { walkedDirs: 0, repositories: 0, problemCount: null, ambiguousLineageCount: null },
        groups: [],
      }),
      'settings.get': () => settings,
      // The core's answer is the whole stored `Settings`, with only the named fields moved.
      'settings.set': ({ patch }) => {
        settings = {
          ...settings,
          effectsTier: patch.effectsTier ?? settings.effectsTier,
          reducedMotionOverride: patch.reducedMotionOverride ?? settings.reducedMotionOverride,
        };
        return settings;
      },
    },
    { effectsTier: launch.boot, effectsTierSource: launch.source ?? 'boot-file' },
  );
  fake.setNow(NOW);
  render(<App deps={fake.deps} />);
  return fake;
}

const rootTier = (): string | null => document.documentElement.getAttribute(EFFECTS_TIER_ATTRIBUTE);

async function openDrawer(): Promise<void> {
  await waitFor(() => {
    expect(document.querySelector('.cdt-shelf')).not.toBeNull();
  });
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  await screen.findByRole('radiogroup', { name: 'Effects' });
}

describe('§11.3a the drawer’s motion rows reach the resolved tier', () => {
  it('a tier picked in the drawer reaches the document element with no relaunch', async () => {
    mount({ boot: 'full', stored: 'full' });
    await openDrawer();
    expect(rootTier()).toBe('full');

    fireEvent.click(screen.getByRole('radio', { name: 'REDUCED' }));
    await waitFor(() => {
      expect(rootTier()).toBe('reduced');
    });

    fireEvent.click(screen.getByRole('radio', { name: 'OFF' }));
    await waitFor(() => {
      expect(rootTier()).toBe('off');
    });
  });

  it('the reduced-motion override clamps a stored full to reduced, and releases it', async () => {
    mount({ boot: 'full', stored: 'full' });
    await openDrawer();

    fireEvent.click(screen.getByRole('switch', { name: OVERRIDE }));
    await waitFor(() => {
      expect(rootTier()).toBe('reduced');
    });

    fireEvent.click(screen.getByRole('switch', { name: OVERRIDE }));
    await waitFor(() => {
      expect(rootTier()).toBe('full');
    });
  });

  it('an override stored before launch clamps from startup, with the drawer never opened', async () => {
    mount({ boot: 'full', stored: 'full', override: true });
    await waitFor(() => {
      expect(rootTier()).toBe('reduced');
    });
  });

  // §11.2a: the database is authoritative and `boot.json` a mirror. A mirror one launch behind —
  // the state every user who changed the tier was left in — must not outlive the core's answer.
  it('the stored tier replaces a stale boot mirror once the core answers', async () => {
    mount({ boot: 'auto', stored: 'off' });
    await waitFor(() => {
      expect(rootTier()).toBe('off');
    });
  });

  // §11.2a: argv and the environment beat `boot.json`, which mirrors the stored setting — so an
  // operator's launch override holds for the session the stored value would otherwise replace.
  it('a launch override from argv outranks the stored tier', async () => {
    const fake = mount({ boot: 'full', source: 'argv', stored: 'off' });
    await waitFor(() => {
      expect(fake.calls.some((call) => call.name === 'settings.get')).toBe(true);
    });
    await waitFor(() => {
      expect(document.querySelector('.cdt-shelf')).not.toBeNull();
    });
    expect(rootTier()).toBe('full');
  });
});
