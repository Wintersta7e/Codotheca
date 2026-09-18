import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectDetail, ProjectId, SceneHash } from '../../generated/protocol';
import type { RendererEvent } from '../../shared/channels';
import { CommandError, ProjectPageDepsContext, type ProjectPageDeps } from './deps';
import { detailFixture, NOW, rowFixture } from './testFixtures';
import { heroHashFrom, redirectTarget, shouldReload, useProjectDetail } from './useProjectDetail';

afterEach(cleanup);

const ID = 7 as unknown as ProjectId;

const ev = (event: string, data: unknown): RendererEvent => ({ topic: 'projects', event, data });

/** The real payload shape: `projects/upserted` carries the whole row, not a bare id. */
const upserted = (id: number): RendererEvent =>
  ev('upserted', { row: rowFixture({ id: id as unknown as ProjectId }) });

describe('which events move this page', () => {
  it('reloads on a fact change for this project', () => {
    expect(shouldReload(upserted(7), ID)).toBe(true);
    expect(shouldReload(ev('condition_changed', { id: 7 }), ID)).toBe(true);
    expect(shouldReload(ev('flags_changed', { id: 7 }), ID)).toBe(true);
  });

  it('ignores every other project on the shelf', () => {
    expect(shouldReload(upserted(8), ID)).toBe(false);
    expect(shouldReload(ev('condition_changed', { id: 8 }), ID)).toBe(false);
  });

  it('ignores topics that are not projects', () => {
    const scan = { topic: 'scan', event: 'progress', data: {} } as RendererEvent;
    expect(shouldReload(scan, ID)).toBe(false);
  });

  it('does not reload the whole detail for art — the bitmap swaps on its own', () => {
    expect(shouldReload(ev('art_ready', { projectId: 7, rendition: 'hero' }), ID)).toBe(false);
  });

  it('survives a payload with no row and no id rather than throwing at the subscriber', () => {
    expect(shouldReload(ev('upserted', {}), ID)).toBe(false);
    expect(shouldReload(ev('upserted', null), ID)).toBe(false);
    expect(shouldReload(ev('upserted', { row: 7 }), ID)).toBe(false);
  });
});

describe('the merge redirect', () => {
  it('follows this project into its survivor', () => {
    expect(redirectTarget(ev('merged', { from: 7, into: 9 }), ID)).toBe(9);
  });

  it('stays put when this project is the survivor', () => {
    expect(redirectTarget(ev('merged', { from: 4, into: 7 }), ID)).toBeNull();
  });

  it('stays put for a merge between two other projects', () => {
    expect(redirectTarget(ev('merged', { from: 4, into: 5 }), ID)).toBeNull();
  });
});

describe('the hero bitmap', () => {
  it('takes a new hash only for this project and only for the hero rendition', () => {
    const at = (over: Record<string, unknown>): SceneHash | null =>
      heroHashFrom(
        ev('art_ready', { projectId: 7, rendition: 'hero', sceneHash: 'aa', ...over }),
        ID,
      );
    expect(at({})).toBe('aa');
    expect(at({ rendition: 'card' })).toBeNull();
    expect(at({ projectId: 8 })).toBeNull();
  });
});

/**
 * The hook itself, not only its predicates. A page that loads once and then ignores every event
 * passes every pure test above and is still broken.
 */
describe('the hook', () => {
  function rig(detail: ProjectDetail = detailFixture()): {
    request: ReturnType<typeof vi.fn>;
    emit: (event: RendererEvent) => void;
    wrapper: (props: { children: ReactNode }) => ReactElement;
  } {
    const handlers = new Set<(event: RendererEvent) => void>();
    const request = vi.fn(() => Promise.resolve(detail));
    const deps: ProjectPageDeps = {
      request: request as unknown as ProjectPageDeps['request'],
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
      installStart: () =>
        Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
      installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
      pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
      openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
      subscribe: (handler) => {
        handlers.add(handler);
        return () => handlers.delete(handler);
      },
      now: () => NOW,
    };
    return {
      request,
      emit: (event) => {
        for (const fn of [...handlers]) fn(event);
      },
      wrapper: ({ children }) => (
        <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
      ),
    };
  }

  it('asks for the detail once and holds the hero hash the row arrived with', async () => {
    const r = rig();
    const view = renderHook(() => useProjectDetail(ID), { wrapper: r.wrapper });
    await waitFor(() => {
      expect(view.result.current.state.kind).toBe('ready');
    });
    expect(r.request).toHaveBeenCalledWith('projects.get', { id: ID });
    expect(view.result.current.heroHash).toBe('aa11bb22');
  });

  it('re-reads the detail when this project changes underneath it', async () => {
    const r = rig();
    const view = renderHook(() => useProjectDetail(ID), { wrapper: r.wrapper });
    await waitFor(() => {
      expect(view.result.current.state.kind).toBe('ready');
    });
    act(() => {
      r.emit(upserted(7));
    });
    await waitFor(() => {
      expect(r.request).toHaveBeenCalledTimes(2);
    });
    act(() => {
      r.emit(upserted(8));
    });
    expect(r.request).toHaveBeenCalledTimes(2);
  });

  it('swaps the hero hash on art_ready without re-reading the detail', async () => {
    const r = rig();
    const view = renderHook(() => useProjectDetail(ID), { wrapper: r.wrapper });
    await waitFor(() => {
      expect(view.result.current.state.kind).toBe('ready');
    });
    act(() => {
      r.emit(ev('art_ready', { projectId: 7, rendition: 'hero', sceneHash: 'ccdd' }));
    });
    await waitFor(() => {
      expect(view.result.current.heroHash).toBe('ccdd');
    });
    expect(r.request).toHaveBeenCalledTimes(1);
  });

  it('reports the closed code rather than a raw core message', async () => {
    const handlers = new Set<(event: RendererEvent) => void>();
    const deps: ProjectPageDeps = {
      request: () =>
        Promise.reject(
          new CommandError({
            code: 'PATH_GONE',
            message: 'stat failed: no such file',
            outcome: null,
            retryable: false,
          }),
        ),
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
      installStart: () =>
        Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
      installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
      pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
      openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
      subscribe: (handler) => {
        handlers.add(handler);
        return () => handlers.delete(handler);
      },
      now: () => NOW,
    };
    const view = renderHook(() => useProjectDetail(ID), {
      wrapper: ({ children }: { children: ReactNode }) => (
        <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
      ),
    });
    await waitFor(() => {
      expect(view.result.current.state).toEqual({ kind: 'failed', code: 'PATH_GONE' });
    });
  });

  it('reports a merge of this project into another so the page can follow it', async () => {
    const r = rig();
    const view = renderHook(() => useProjectDetail(ID), { wrapper: r.wrapper });
    await waitFor(() => {
      expect(view.result.current.state.kind).toBe('ready');
    });
    act(() => {
      r.emit(ev('merged', { from: 7, into: 9 }));
    });
    await waitFor(() => {
      expect(view.result.current.redirectedTo).toBe(9);
    });
  });
});
