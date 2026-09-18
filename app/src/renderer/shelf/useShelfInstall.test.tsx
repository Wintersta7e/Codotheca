/**
 * §24.3d's shelf half. The subject is **how many round trips a library costs**: a preview is
 * asked for once per project and only for the tiles that are mounted, which on a virtualized
 * grid is the ones near the viewport.
 */
import { act, cleanup, renderHook, waitFor, type RenderHookResult } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { InstallPreview, ProjectId, RootId, Settings } from '../../generated/protocol';
import type { AppDeps } from '../app/deps';
import { useShelfInstall, type ShelfInstall } from './useShelfInstall';

afterEach(cleanup);

const ROOT = 3 as unknown as RootId;
const A = 7 as unknown as ProjectId;
const B = 8 as unknown as ProjectId;

const settings = (installRootId: RootId | null): Settings => ({
  effectsTier: 'full',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  contentScanEnabled: false,
  logLevel: 'info',
  installRootId,
});

const preview = (display: string): InstallPreview =>
  ({ destination: { display }, refusedBecause: null }) as unknown as InstallPreview;

function draw(
  stored: RootId | null,
  installStart: ReturnType<typeof vi.fn> = vi.fn(() =>
    Promise.resolve({ kind: 'started', start: { runId: 5, refusedBecause: null } }),
  ),
): {
  view: RenderHookResult<ShelfInstall, unknown>;
  request: ReturnType<typeof vi.fn>;
  installStart: ReturnType<typeof vi.fn>;
} {
  const request = vi.fn((name: string) => {
    if (name === 'settings.get') return Promise.resolve(settings(stored));
    if (name === 'install.preview') return Promise.resolve(preview('~/work/aurora'));
    return Promise.resolve({});
  });
  const deps = {
    request: request as unknown as AppDeps['request'],
    installStart: installStart as unknown as AppDeps['installStart'],
  } as AppDeps;
  const view = renderHook(() => useShelfInstall(deps));
  return { view, request, installStart };
}

const previewCalls = (request: ReturnType<typeof vi.fn>): number =>
  request.mock.calls.filter((c) => (c as unknown[])[0] === 'install.preview').length;

describe('the shelf-wide install offer', () => {
  it('asks for nothing until a tile demands it', async () => {
    const r = draw(ROOT);
    await waitFor(() => {
      expect(r.request).toHaveBeenCalledWith('settings.get', {});
    });
    expect(previewCalls(r.request)).toBe(0);
    expect(r.view.result.current.previews.size).toBe(0);
  });

  it('asks once per project however often the tile re-renders', async () => {
    const r = draw(ROOT);
    await waitFor(() => {
      expect(r.request).toHaveBeenCalledWith('settings.get', {});
    });

    act(() => {
      r.view.result.current.need(A);
      r.view.result.current.need(A);
      r.view.result.current.need(B);
    });
    await waitFor(() => {
      expect(r.view.result.current.previews.size).toBe(2);
    });
    expect(previewCalls(r.request)).toBe(2);

    act(() => {
      r.view.result.current.need(A);
    });
    expect(previewCalls(r.request)).toBe(2);
  });

  it('offers nothing at all where no destination has been chosen', async () => {
    const r = draw(null);
    await waitFor(() => {
      expect(r.request).toHaveBeenCalledWith('settings.get', {});
    });
    // The settings read having *resolved*, not merely been issued. An absence asserted before
    // the read lands passes whether the rule holds or not — the same gap that made the sibling
    // test above flake, arriving here as a bar that proves nothing rather than as a failure.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    act(() => {
      r.view.result.current.need(A);
      r.view.result.current.start(A);
    });
    expect(previewCalls(r.request)).toBe(0);
    expect(r.installStart).not.toHaveBeenCalled();
  });

  it('serves a tile that demanded before the destination was known', async () => {
    // The first paint: every tile on screen mounts before `settings.get` answers. A demand
    // dropped there is never repeated — the tile's effect does not run again — so a shelf would
    // offer no install at all until a card was scrolled out of view and back.
    const r = draw(ROOT);
    act(() => {
      r.view.result.current.need(A);
    });
    await waitFor(() => {
      expect(r.view.result.current.previews.get(A)).toBeDefined();
    });
    expect(previewCalls(r.request)).toBe(1);
  });

  it('starts with two opaque ids and re-reads that project afterwards', async () => {
    const r = draw(ROOT);
    act(() => {
      r.view.result.current.need(A);
    });
    await waitFor(() => {
      expect(previewCalls(r.request)).toBe(1);
    });

    act(() => {
      r.view.result.current.start(A);
    });
    await waitFor(() => {
      expect(r.installStart).toHaveBeenCalledWith(A, ROOT);
    });
    await waitFor(() => {
      expect(previewCalls(r.request)).toBe(2);
    });
  });

  it('runs one start per project however fast it is pressed twice', async () => {
    const installStart = vi.fn(() => new Promise<never>(() => undefined));
    const r = draw(ROOT, installStart);

    // **Waits for the preview, not for the `settings.get` call.** The call having been *made* is
    // not the destination having *landed*, and a start before it lands is dropped — correctly,
    // because the button that fires it only renders once a preview exists. Waiting on the call
    // made this pass alone and fail under the parallel gate, which is the flake shape exactly.
    act(() => {
      r.view.result.current.need(A);
    });
    await waitFor(() => {
      expect(r.view.result.current.previews.get(A)).toBeDefined();
    });

    act(() => {
      r.view.result.current.start(A);
      r.view.result.current.start(A);
    });
    expect(installStart).toHaveBeenCalledTimes(1);
  });
});
