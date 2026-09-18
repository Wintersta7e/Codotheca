import { cleanup, renderHook, waitFor, type RenderHookResult } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ArtState, SceneHash } from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { NOW } from '../testFixtures';
import { useHeroArt } from './useHeroArt';

afterEach(cleanup);

const hash = (value: string): SceneHash => value as SceneHash;

function rig(request: ReturnType<typeof vi.fn>): {
  wrapper: (props: { children: ReactNode }) => ReactElement;
} {
  const deps: ProjectPageDeps = {
    request: request as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    installStart: () =>
      Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
    installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
    pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  return {
    wrapper: ({ children }) => (
      <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
    ),
  };
}

function draw(
  request: ReturnType<typeof vi.fn>,
  initial: { hash: SceneHash | null; artState: ArtState },
): RenderHookResult<string, { h: SceneHash | null; s: ArtState }> {
  const { wrapper } = rig(request);
  return renderHook(({ h, s }: { h: SceneHash | null; s: ArtState }) => useHeroArt(h, s), {
    wrapper,
    initialProps: { h: initial.hash, s: initial.artState },
  });
}

describe('the hero address', () => {
  it('asks the core for it and never builds one, naming the hero rendition', async () => {
    const request = vi.fn(() => Promise.resolve('codotheca://art/aa/hero'));
    const view = draw(request, { hash: hash('aa'), artState: 'ready' });
    await waitFor(() => {
      expect(view.result.current).toBe('codotheca://art/aa/hero');
    });
    expect(request).toHaveBeenCalledWith('art.url', { hash: 'aa', rendition: 'hero' });
  });

  it('holds the answered address while the next hash is being answered', async () => {
    let release: ((url: string) => void) | null = null;
    const request = vi.fn((_name: string, args: { hash: string }) => {
      if (args.hash === 'aa') return Promise.resolve('url-a');
      return new Promise<string>((resolve) => {
        release = resolve;
      });
    });
    const view = draw(request, { hash: hash('aa'), artState: 'ready' });
    await waitFor(() => {
      expect(view.result.current).toBe('url-a');
    });

    view.rerender({ h: hash('bb'), s: 'ready' });
    // The plate must not flash between two ready bitmaps.
    expect(view.result.current).toBe('url-a');
    if (release === null) throw new Error('the second request was never issued');
    (release as (url: string) => void)('url-b');
    await waitFor(() => {
      expect(view.result.current).toBe('url-b');
    });
  });

  it('asks for no address at all when the core has already failed to render one', () => {
    const request = vi.fn(() => Promise.resolve('nope'));
    const view = draw(request, { hash: hash('aa'), artState: 'failed' });
    expect(request).not.toHaveBeenCalled();
    expect(view.result.current).toBe('');
  });

  it('asks for no address when there is no scene hash yet', () => {
    const request = vi.fn(() => Promise.resolve('nope'));
    const view = draw(request, { hash: null, artState: 'pending' });
    expect(request).not.toHaveBeenCalled();
    expect(view.result.current).toBe('');
  });

  it('keeps whatever is on screen when the address cannot be resolved', async () => {
    const request = vi.fn((_name: string, args: { hash: string }) =>
      args.hash === 'aa' ? Promise.resolve('url-a') : Promise.reject(new Error('gone')),
    );
    const view = draw(request, { hash: hash('aa'), artState: 'ready' });
    await waitFor(() => {
      expect(view.result.current).toBe('url-a');
    });
    view.rerender({ h: hash('bb'), s: 'ready' });
    await waitFor(() => {
      expect(request).toHaveBeenCalledTimes(2);
    });
    expect(view.result.current).toBe('url-a');
  });

  it('treats an empty answer as no address rather than as one', async () => {
    const request = vi.fn(() => Promise.resolve(''));
    const view = draw(request, { hash: hash('aa'), artState: 'ready' });
    await waitFor(() => {
      expect(request).toHaveBeenCalledTimes(1);
    });
    expect(view.result.current).toBe('');
  });
});
