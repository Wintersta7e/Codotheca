/**
 * §24.8's verdict as the page holds it. The subject of every test here is *when* the pre-flight
 * runs, because the criterion §24.7C states is about cost and currency and not about rendering.
 */
import { act, cleanup, renderHook, waitFor, type RenderHookResult } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { LocationId, UninstallVerdict } from '../../../generated/protocol';
import type { UninstallReply } from '../../../shared/channels';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { NOW } from '../testFixtures';
import { useUninstallOffer, type UninstallOffer } from './useUninstall';

afterEach(cleanup);

const id = (value: number): LocationId => value as unknown as LocationId;

function verdict(over: Partial<UninstallVerdict> = {}): UninstallVerdict {
  return {
    disposition: 'safe',
    blockers: [],
    remoteVerifiedAt: null,
    trashAvailable: true,
    computedAt: NOW,
    ...over,
  };
}

interface Rig {
  view: RenderHookResult<UninstallOffer, { locationId: LocationId | null }>;
  request: ReturnType<typeof vi.fn>;
  uninstall: ReturnType<typeof vi.fn>;
  onChanged: ReturnType<typeof vi.fn>;
}

function draw(
  request: ReturnType<typeof vi.fn>,
  uninstall: ReturnType<typeof vi.fn> = vi.fn(() =>
    Promise.resolve({ kind: 'uninstalled', location: {} } as UninstallReply),
  ),
  locationId: LocationId | null = id(4),
): Rig {
  const onChanged = vi.fn();
  const deps: ProjectPageDeps = {
    request: request as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: uninstall as unknown as ProjectPageDeps['uninstall'],
    installStart: () =>
      Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
    installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
    pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  const wrapper = ({ children }: { children: ReactNode }): ReactElement => (
    <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
  );
  const view = renderHook(
    ({ locationId: held }: { locationId: LocationId | null }) => useUninstallOffer(held, onChanged),
    { wrapper, initialProps: { locationId } },
  );
  return { view, request, uninstall, onChanged };
}

const answering = (value: UninstallVerdict): ReturnType<typeof vi.fn> =>
  vi.fn(() => Promise.resolve(value));

describe('the pre-flight runs on a press and at no other time', () => {
  it('issues no command on mount, because a fetch is not free', () => {
    const r = draw(answering(verdict()));
    expect(r.request).not.toHaveBeenCalled();
    // `undefined` is *not offering it*, which is what keeps the control off the rail entirely.
    expect(r.view.result.current.verdict).toBeUndefined();
  });

  it('issues no command when the location the page is showing changes', () => {
    const r = draw(answering(verdict()));
    r.view.rerender({ locationId: id(9) });
    expect(r.request).not.toHaveBeenCalled();
  });

  it('asks once when the affordance opens, and shows checking until it answers', async () => {
    const r = draw(answering(verdict()));

    act(() => {
      r.view.result.current.open();
    });
    // `null` is *in flight* and is a third state, never an enabled button taken away later.
    expect(r.view.result.current.verdict).toBeNull();
    expect(r.request).toHaveBeenCalledTimes(1);
    expect(r.request).toHaveBeenCalledWith('locations.uninstallPreflight', { locationId: id(4) });

    await waitFor(() => {
      expect(r.view.result.current.verdict).not.toBeNull();
    });
    expect(r.view.result.current.verdict?.disposition).toBe('safe');
  });

  it('renders the core disposition rather than composing one', async () => {
    const answer = verdict({ disposition: 'unknown', blockers: ['remote_unreachable'] });
    const r = draw(answering(answer));

    act(() => {
      r.view.result.current.open();
    });
    await waitFor(() => {
      expect(r.view.result.current.verdict?.disposition).toBe('unknown');
    });
    expect(r.view.result.current.verdict?.blockers).toEqual(['remote_unreachable']);
  });

  it('closes the offer when the pre-flight does not answer, stating nothing it cannot know', async () => {
    const request = vi.fn(() => Promise.reject(new Error('core is down')));
    const r = draw(request);

    act(() => {
      r.view.result.current.open();
    });
    await waitFor(() => {
      expect(r.view.result.current.verdict).toBeUndefined();
    });
  });

  it('drops a reply that arrives after the page moved to another copy', async () => {
    let settle: ((value: UninstallVerdict) => void) | null = null;
    const request = vi.fn(
      () =>
        new Promise<UninstallVerdict>((resolve) => {
          settle = resolve;
        }),
    );
    const r = draw(request);

    act(() => {
      r.view.result.current.open();
    });
    r.view.rerender({ locationId: id(9) });
    act(() => {
      settle?.(verdict());
    });

    await waitFor(() => {
      expect(r.view.result.current.verdict).toBeUndefined();
    });
  });
});

describe('the removal', () => {
  it('sends a location id and no verdict token', async () => {
    const r = draw(answering(verdict()));

    act(() => {
      r.view.result.current.remove();
    });
    await waitFor(() => {
      expect(r.onChanged).toHaveBeenCalledTimes(1);
    });
    expect(r.uninstall).toHaveBeenCalledWith(id(4));
    expect(r.uninstall.mock.calls[0]).toHaveLength(1);
  });

  it('re-reads the verdict when the core refuses, never showing its diagnostic', async () => {
    // §24.8: the core recomputes inside the call and refuses if the answer moved. `refused`
    // carries the core's `message`, which §2.4 forbids rendering — so the honest answer is the
    // fresh verdict, asked for again.
    const blocked = verdict({ disposition: 'blocked', blockers: ['uncommitted_changes'] });
    const request = answering(blocked);
    const uninstall = vi.fn(() =>
      Promise.resolve({ kind: 'refused', verdict: 'verdict changed' } as UninstallReply),
    );
    const r = draw(request, uninstall);

    act(() => {
      r.view.result.current.remove();
    });
    await waitFor(() => {
      expect(r.view.result.current.verdict?.disposition).toBe('blocked');
    });
    expect(request).toHaveBeenCalledWith('locations.uninstallPreflight', { locationId: id(4) });
    expect(r.onChanged).not.toHaveBeenCalled();
  });

  it('runs once even if pressed twice within a tick', () => {
    const uninstall = vi.fn(
      () => new Promise<UninstallReply>(() => undefined), // never settles
    );
    const r = draw(answering(verdict()), uninstall);

    act(() => {
      r.view.result.current.remove();
      r.view.result.current.remove();
    });
    expect(uninstall).toHaveBeenCalledTimes(1);
  });

  it('does nothing at all when the page is showing no copy', () => {
    const r = draw(answering(verdict()), undefined, null);

    act(() => {
      r.view.result.current.open();
      r.view.result.current.remove();
    });
    expect(r.request).not.toHaveBeenCalled();
    expect(r.uninstall).not.toHaveBeenCalled();
  });
});
