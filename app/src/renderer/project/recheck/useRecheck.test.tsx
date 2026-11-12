import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectDetail, ProjectId } from '../../../generated/protocol';
import type { RendererEvent } from '../../../shared/channels';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { ProjectPageView } from '../ProjectPage';
import { detailFixture, NOW, rowFixture } from '../testFixtures';

afterEach(cleanup);

const ID = 7 as unknown as ProjectId;

/** A page's whole outside: `request` records every command, `emit` plays a core event. */
function rig(detail: ProjectDetail = detailFixture()): {
  sent: (name: string) => unknown[];
  emit: (event: RendererEvent) => void;
  page: (id: ProjectId) => ReactElement;
} {
  const handlers = new Set<(event: RendererEvent) => void>();
  const calls: [string, unknown][] = [];
  const request = vi.fn((name: string, args: unknown) => {
    calls.push([name, args]);
    if (name === 'projects.get') return Promise.resolve(detail);
    if (name === 'projects.recheck') return Promise.resolve({ queued: true });
    if (name === 'art.url') return Promise.resolve('codotheca://art/aa/hero');
    return Promise.resolve({});
  });
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
    sent: (name) => calls.filter(([n]) => n === name).map(([, args]) => args),
    emit: (event) => {
      for (const fn of [...handlers]) fn(event);
    },
    page: (id) => (
      <ProjectPageDepsContext.Provider value={deps}>
        <ProjectPageView projectId={id} onBack={vi.fn()} onOpenProject={vi.fn()} />
      </ProjectPageDepsContext.Provider>
    ),
  };
}

describe('the page-mount re-check (§54.4 caller 2)', () => {
  it('AC-P4-54-2 mounting the page sends exactly one projects.recheck', async () => {
    const r = rig();
    render(r.page(ID));
    await screen.findByRole('tab', { name: 'OVERVIEW' });
    expect(r.sent('projects.recheck')).toEqual([{ id: ID }]);
  });

  it('AC-P4-54-2 an upserted-driven projects.get reload sends none', async () => {
    const r = rig();
    render(r.page(ID));
    await screen.findByRole('tab', { name: 'OVERVIEW' });
    expect(r.sent('projects.get')).toHaveLength(1);
    act(() => {
      r.emit({
        topic: 'projects',
        event: 'upserted',
        data: { row: rowFixture({ id: ID }) },
      });
    });
    await waitFor(() => {
      expect(r.sent('projects.get')).toHaveLength(2);
    });
    expect(r.sent('projects.recheck')).toEqual([{ id: ID }]);
  });

  it('changing the project id sends one for the new id', async () => {
    const r = rig();
    const view = render(r.page(ID));
    await screen.findByRole('tab', { name: 'OVERVIEW' });
    const next = 8 as unknown as ProjectId;
    view.rerender(r.page(next));
    await waitFor(() => {
      expect(r.sent('projects.recheck')).toEqual([{ id: ID }, { id: next }]);
    });
  });
});
