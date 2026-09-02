import { act, render } from '@testing-library/react';
import type { ReactElement } from 'react';
import { describe, expect, it } from 'vitest';

import type {
  LocationId,
  ProjectId,
  SessionId,
  SessionRef,
  TargetId,
} from '../../generated/protocol';
import type { AppDeps } from './deps';
import { fakeAppDeps, type FakeAppDeps } from './testDeps';
import { useSessions } from './useSessions';

function session(over: Partial<SessionRef> = {}): SessionRef {
  return {
    id: 1 as SessionId,
    projectId: 7 as ProjectId,
    locationId: 3 as LocationId,
    targetId: 2 as TargetId,
    startedAt: 1_700_000_000,
    endedAt: null,
    creditedSeconds: 0,
    closeReason: null,
    ...over,
  };
}

function Probe({
  deps,
  seen,
}: {
  deps: AppDeps;
  seen: ReadonlyMap<ProjectId, SessionRef>[];
}): ReactElement {
  seen.push(useSessions(deps));
  return <div />;
}

function mount(fake: FakeAppDeps): { last: () => ReadonlyMap<ProjectId, SessionRef> } {
  const seen: ReadonlyMap<ProjectId, SessionRef>[] = [];
  render(<Probe deps={fake.deps} seen={seen} />);
  return {
    last: () => {
      const state = seen.at(-1);
      if (state === undefined) throw new Error('the hook rendered nothing');
      return state;
    },
  };
}

describe('useSessions', () => {
  it('is empty before anything starts, and a project with no session is absent', () => {
    const view = mount(fakeAppDeps());
    expect(view.last().size).toBe(0);
    // §7.8's live tile is present or it is not. A zero-length entry would draw a bench row for
    // a project nobody launched.
    expect(view.last().has(7 as ProjectId)).toBe(false);
    expect(view.last().get(7 as ProjectId)).toBeUndefined();
  });

  it('holds a started session against its project and drops it when it ends', () => {
    const fake = fakeAppDeps();
    const view = mount(fake);

    act(() => {
      fake.emit({ topic: 'session', event: 'started', data: { session: session() } });
    });
    expect(view.last().get(7 as ProjectId)?.id).toBe(1);

    act(() => {
      fake.emit({
        topic: 'session',
        event: 'ended',
        data: { session: session({ endedAt: 1_700_000_600, closeReason: 'process_exit' }) },
      });
    });
    expect(view.last().has(7 as ProjectId)).toBe(false);
  });

  it('a closed segment credits the live session rather than ending it', () => {
    const fake = fakeAppDeps();
    const view = mount(fake);
    act(() => {
      fake.emit({ topic: 'session', event: 'started', data: { session: session() } });
    });
    act(() => {
      fake.emit({
        topic: 'session',
        event: 'segment_closed',
        data: {
          sessionId: 1,
          projectId: 7,
          startedAt: 1_700_000_000,
          endedAt: 1_700_000_300,
          creditedSeconds: 300,
          closedBy: 'idle',
          sessionCreditedSeconds: 300,
        },
      });
    });
    expect(view.last().get(7 as ProjectId)?.creditedSeconds).toBe(300);
    expect(view.last().has(7 as ProjectId)).toBe(true);
  });

  it('a segment for a session it never saw start adds nothing', () => {
    const fake = fakeAppDeps();
    const view = mount(fake);
    act(() => {
      fake.emit({
        topic: 'session',
        event: 'segment_closed',
        data: {
          sessionId: 9,
          projectId: 11,
          startedAt: 1,
          endedAt: 2,
          creditedSeconds: 1,
          closedBy: 'idle',
          sessionCreditedSeconds: 1,
        },
      });
    });
    // Half a session assembled from a segment would put a live tile on a project with none.
    expect(view.last().size).toBe(0);
  });
});
