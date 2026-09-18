/**
 * §24.3's offer as a surface holds it. The subject is **what is asked and with what** — the
 * destination is a `RootId` and never a path, and the composed display form comes back from the
 * core rather than being assembled here.
 */
import { act, cleanup, renderHook, waitFor, type RenderHookResult } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { InstallPreview, ProjectId, Root, RootId, Settings } from '../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../project/deps';
import { NOW } from '../project/testFixtures';
import { useInstallOffer, type InstallOffer } from './useInstallOffer';

afterEach(cleanup);

const PROJECT = 7 as unknown as ProjectId;
const ROOT = 3 as unknown as RootId;

const settings = (installRootId: RootId | null): Settings => ({
  effectsTier: 'full',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId,
});

const root = (id: number, pathDisplay: string): Root =>
  ({
    id: id as unknown as RootId,
    pathDisplay,
    kind: 'native',
    distro: '',
    enabled: true,
    descendIntoRepos: false,
    provenance: 'chosen',
    state: 'ok',
    addedAt: NOW,
    projectCount: null,
  }) as unknown as Root;

const preview = (display: string): InstallPreview =>
  ({
    destination: { display },
    refusedBecause: null,
  }) as unknown as InstallPreview;

interface Rig {
  view: RenderHookResult<InstallOffer, { projectId: ProjectId | null }>;
  request: ReturnType<typeof vi.fn>;
  installStart: ReturnType<typeof vi.fn>;
  pickRoot: ReturnType<typeof vi.fn>;
}

function draw(
  request: ReturnType<typeof vi.fn>,
  over: {
    installStart?: ReturnType<typeof vi.fn>;
    pickRoot?: ReturnType<typeof vi.fn>;
    projectId?: ProjectId | null;
  } = {},
): Rig {
  const installStart =
    over.installStart ??
    vi.fn(() => Promise.resolve({ kind: 'started', start: { runId: 5, refusedBecause: null } }));
  const pickRoot = over.pickRoot ?? vi.fn(() => Promise.resolve({ kind: 'cancelled' }));
  const deps: ProjectPageDeps = {
    request: request as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    installStart: installStart as unknown as ProjectPageDeps['installStart'],
    installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
    pickRoot: pickRoot as unknown as ProjectPageDeps['pickRoot'],
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  const wrapper = ({ children }: { children: ReactNode }): ReactElement => (
    <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
  );
  const view = renderHook(
    ({ projectId }: { projectId: ProjectId | null }) => useInstallOffer(projectId),
    { wrapper, initialProps: { projectId: over.projectId === undefined ? PROJECT : null } },
  );
  return { view, request, installStart, pickRoot };
}

/**
 * Stateful, because a `settings.set` the next `settings.get` does not reflect is a core no
 * product has. The first version returned a constant and the choice appeared to vanish.
 */
const answering = (
  stored: RootId | null,
  roots: readonly Root[] = [root(3, '~/work'), root(4, '~/src')],
): ReturnType<typeof vi.fn> => {
  let held = stored;
  return vi.fn((name: string, args: unknown) => {
    if (name === 'settings.get') return Promise.resolve(settings(held));
    if (name === 'roots.list') return Promise.resolve(roots);
    if (name === 'install.preview') return Promise.resolve(preview('~/work/aurora'));
    if (name === 'settings.set') {
      held = (args as { patch: { installRootId: RootId | null } }).patch.installRootId;
      return Promise.resolve(settings(held));
    }
    return Promise.resolve({});
  });
};

describe('choosing where a clone lands', () => {
  it('offers the roots that exist, and previews nothing, when none is stored', async () => {
    const r = draw(answering(null));

    await waitFor(() => {
      expect(r.view.result.current.roots).not.toBeNull();
    });
    expect(r.view.result.current.roots?.map((x) => x.pathDisplay)).toEqual(['~/work', '~/src']);
    // §24.3a: `install.preview` takes a RootId. With none there is nothing to ask.
    expect(r.request.mock.calls.map((c) => String((c as unknown[])[0]))).not.toContain(
      'install.preview',
    );
  });

  it('stores the choice as a setting and previews against it', async () => {
    const r = draw(answering(null));
    await waitFor(() => {
      expect(r.view.result.current.roots).not.toBeNull();
    });

    act(() => {
      r.view.result.current.choose(ROOT);
    });

    await waitFor(() => {
      expect(r.request).toHaveBeenCalledWith('install.preview', {
        projectId: PROJECT,
        rootId: ROOT,
      });
    });
    const patch = r.request.mock.calls.find((c) => (c as unknown[])[0] === 'settings.set');
    expect((patch?.[1] as { patch: { installRootId: RootId } }).patch.installRootId).toBe(ROOT);
  });

  it('sends an id and never a path display, on either call', async () => {
    const r = draw(answering(ROOT));
    await waitFor(() => {
      expect(r.view.result.current.preview).not.toBeNull();
    });

    const asked = r.request.mock.calls.filter((c) => (c as unknown[])[0] === 'install.preview');
    for (const call of asked) {
      expect(Object.keys((call as unknown[])[1] as object).sort()).toEqual(['projectId', 'rootId']);
    }
  });

  it('re-reads after the shell adds a folder, and does nothing when it is cancelled', async () => {
    const pickRoot = vi.fn(() => Promise.resolve({ kind: 'cancelled' }));
    const r = draw(answering(null), { pickRoot });
    await waitFor(() => {
      expect(r.view.result.current.roots).not.toBeNull();
    });
    const before = r.request.mock.calls.length;

    act(() => {
      r.view.result.current.addFolder();
    });
    await waitFor(() => {
      expect(pickRoot).toHaveBeenCalledWith(false);
    });
    expect(r.request.mock.calls.length).toBe(before);
  });
});

describe('the preview and the start', () => {
  it('renders the destination the core composed, not one it built', async () => {
    const r = draw(answering(ROOT));
    await waitFor(() => {
      expect(r.view.result.current.preview?.destination?.display).toBe('~/work/aurora');
    });
    expect(r.view.result.current.roots).toBeNull();
  });

  it('starts with two opaque ids', async () => {
    const r = draw(answering(ROOT));
    await waitFor(() => {
      expect(r.view.result.current.preview).not.toBeNull();
    });

    act(() => {
      r.view.result.current.start();
    });
    await waitFor(() => {
      expect(r.installStart).toHaveBeenCalledWith(PROJECT, ROOT);
    });
  });

  it('runs one start however fast it is pressed twice', async () => {
    const installStart = vi.fn(() => new Promise<never>(() => undefined));
    const r = draw(answering(ROOT), { installStart });
    await waitFor(() => {
      expect(r.view.result.current.preview).not.toBeNull();
    });

    act(() => {
      r.view.result.current.start();
      r.view.result.current.start();
    });
    expect(installStart).toHaveBeenCalledTimes(1);
  });

  it('re-reads the preview when the start is refused, rather than composing the reason', async () => {
    const installStart = vi.fn(() =>
      Promise.resolve({
        kind: 'started',
        start: { runId: null, refusedBecause: 'destination_exists' },
      }),
    );
    const r = draw(answering(ROOT), { installStart });
    await waitFor(() => {
      expect(r.view.result.current.preview).not.toBeNull();
    });
    const before = r.request.mock.calls.filter(
      (c) => (c as unknown[])[0] === 'install.preview',
    ).length;

    act(() => {
      r.view.result.current.start();
    });
    await waitFor(() => {
      const after = r.request.mock.calls.filter(
        (c) => (c as unknown[])[0] === 'install.preview',
      ).length;
      expect(after).toBeGreaterThan(before);
    });
  });

  it('asks for nothing at all on a surface that is not offering it', () => {
    const r = draw(answering(ROOT), { projectId: null });
    expect(r.request).not.toHaveBeenCalled();
    expect(r.view.result.current.preview).toBeUndefined();
  });
});
