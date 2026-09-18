import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ProjectId, Problems, ScanRunId, Settings } from '../../generated/protocol';
import { SETTINGS_ROWS } from '../settings/Drawer';
import { makeProjectRow } from '../testing/projectRow';
import { SurfaceHost, type SurfaceHostProps } from './SurfaceHost';
import { fakeAppDeps, type FakeReplies } from './testDeps';

afterEach(() => {
  cleanup();
});

const settings: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
};

const problems: Problems = {
  runId: 4 as ScanRunId,
  header: { walkedDirs: 10, repositories: 3, problemCount: 1, ambiguousLineageCount: 0 },
  groups: [],
};

const DRAWER_REPLIES: FakeReplies = {
  'settings.get': () => settings,
  'roots.list': () => [],
  'targets.list': () => ({ resolved: null, rows: [] }),
  'identity.list': () => [],
  // [p2] §20.12's panel is a filled slot now, and it reads on mount. `null` for the org list is
  // *unknown*, which is what an account-less drawer should carry.
  'accounts.list': () => [],
  'accounts.orgs': () => null,
};

function mount(over: Partial<SurfaceHostProps> = {}, replies: FakeReplies = DRAWER_REPLIES): void {
  const fake = fakeAppDeps(replies);
  render(
    <SurfaceHost
      deps={fake.deps}
      tier="off"
      failure={null}
      settingsOpen={false}
      onCloseSettings={vi.fn()}
      summaryOpen={false}
      onCloseSummary={vi.fn()}
      problems={null}
      onProblemsChanged={vi.fn()}
      rows={[]}
      liveSessionProjectIds={new Set()}
      onOpenProject={vi.fn()}
      onLaunch={vi.fn()}
      {...over}
    />,
  );
}

describe('SurfaceHost', () => {
  it('draws no overlay at rest', () => {
    mount();
    expect(screen.queryByText('SETTINGS')).toBeNull();
    expect(document.querySelector('.cdt-quick-switch')).toBeNull();
  });

  it('opens the drawer and closes it without navigating', async () => {
    const onCloseSettings = vi.fn();
    const onOpenProject = vi.fn();
    mount({ settingsOpen: true, onCloseSettings, onOpenProject });
    await waitFor(() => {
      expect(screen.getByText('SETTINGS')).toBeTruthy();
    });

    const panel = document.querySelector('[role="dialog"]') ?? document.body;
    fireEvent.keyDown(panel, { key: 'Escape', code: 'Escape' });
    expect(onCloseSettings).toHaveBeenCalledTimes(1);
    // §11.3a: the drawer is an overlay, not a route. Closing it must leave the surface behind
    // it exactly where it was.
    expect(onOpenProject).not.toHaveBeenCalled();
  });

  it('draws no row for a settings slot nothing implements', async () => {
    mount({ settingsOpen: true });
    await waitFor(() => {
      expect(screen.getByText('SETTINGS')).toBeTruthy();
    });

    // §11.3a forbids a control with nothing behind it by name. With every slot unfilled, the
    // drawer draws no control for a host-backed row: the two whole-row cases are absent, and the
    // launch-target rows keep their statement and lose their button. A no-op filler would draw
    // all three and look like a working switch.
    const hostRows = SETTINGS_ROWS.filter((row) => row.backing.kind === 'host');
    expect(hostRows.length).toBeGreaterThan(0);
    expect(screen.queryByText('HIDE A PROJECT')).toBeNull();
    expect(screen.queryByText('+ ADD AN ADDRESS')).toBeNull();
    expect(screen.queryByText('CHOOSE')).toBeNull();
    expect(screen.queryByText('CHANGE')).toBeNull();
  });

  it('shows the scan summary only when there is a report to show', async () => {
    mount({ summaryOpen: true, problems: null });
    expect(screen.queryByText(/PROBLEM/u)).toBeNull();

    mount({ summaryOpen: true, problems });
    await waitFor(() => {
      expect(document.body.textContent).toContain('1');
    });
  });

  it('draws the failure window only for a fact, and full screen when it does', () => {
    mount({ failure: null });
    expect(screen.queryByText(/THIS LIBRARY WAS WRITTEN BY A NEWER CODOTHECA/u)).toBeNull();

    mount({ failure: { kind: 'schema_from_future', onDisk: 9, supported: 5 } });
    expect(screen.getByText('THIS LIBRARY WAS WRITTEN BY A NEWER CODOTHECA')).toBeTruthy();
  });

  it('names the log the shell passed rather than a path it invented', () => {
    const fake = fakeAppDeps(DRAWER_REPLIES, { logPath: '/var/log/codotheca.log' });
    render(
      <SurfaceHost
        deps={fake.deps}
        tier="off"
        failure={{ kind: 'schema_from_future', onDisk: 9, supported: 5 }}
        settingsOpen={false}
        onCloseSettings={vi.fn()}
        summaryOpen={false}
        onCloseSummary={vi.fn()}
        problems={null}
        onProblemsChanged={vi.fn()}
        rows={[]}
        liveSessionProjectIds={new Set()}
        onOpenProject={vi.fn()}
        onLaunch={vi.fn()}
      />,
    );
    expect(document.body.textContent).toContain('/var/log/codotheca.log');
  });

  it('binds the palette to the shell channel that opens it', () => {
    const fake = fakeAppDeps(DRAWER_REPLIES);
    render(
      <SurfaceHost
        deps={fake.deps}
        tier="off"
        failure={null}
        settingsOpen={false}
        onCloseSettings={vi.fn()}
        summaryOpen={false}
        onCloseSummary={vi.fn()}
        problems={null}
        onProblemsChanged={vi.fn()}
        rows={[makeProjectRow({ id: 1 as ProjectId, name: 'one' })]}
        liveSessionProjectIds={new Set()}
        onOpenProject={vi.fn()}
        onLaunch={vi.fn()}
      />,
    );
    expect(document.querySelector('[role="combobox"]')).toBeNull();

    // §8.6: the resident chord shows the window and opens the palette. Plan 15's Task 10 was the
    // mount, and it had no host until now.
    act(() => {
      fake.openPalette();
    });
    expect(document.querySelector('[role="combobox"]')).not.toBeNull();
  });
});
