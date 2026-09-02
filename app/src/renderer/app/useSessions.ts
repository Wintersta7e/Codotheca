/**
 * The `session` topic as a map from project to its one open session.
 *
 * §7.8's live tile is present or it is not: a project with no open session is **absent** from
 * this map, never a zero-length entry. A zero-length entry would draw a bench row reading
 * `AT THE BENCH · 0m` for a project nobody launched, which is the wrong sentence twice over.
 */
import { useEffect, useState } from 'react';

import type { ProjectId, SessionRef } from '../../generated/protocol.js';
import type { RendererEvent } from '../../shared/channels.js';
import type { AppDeps } from './deps.js';

interface Payload {
  readonly session?: unknown;
  readonly projectId?: unknown;
  readonly sessionId?: unknown;
  readonly sessionCreditedSeconds?: unknown;
}

const EMPTY: ReadonlyMap<ProjectId, SessionRef> = new Map();

export function applySessionEvent(
  live: ReadonlyMap<ProjectId, SessionRef>,
  event: RendererEvent,
): ReadonlyMap<ProjectId, SessionRef> {
  if (event.topic !== 'session') return live;
  const data = (event.data ?? {}) as Payload;

  if (event.event === 'started') {
    const session = data.session as SessionRef | undefined;
    if (session === undefined) return live;
    const next = new Map(live);
    next.set(session.projectId, session);
    return next;
  }

  if (event.event === 'ended') {
    const session = data.session as SessionRef | undefined;
    if (session === undefined) return live;
    if (!live.has(session.projectId)) return live;
    const next = new Map(live);
    next.delete(session.projectId);
    return next;
  }

  if (event.event === 'segment_closed') {
    // §9: a closed segment credits a session that is still open. It never opens one — a session
    // this map never saw start is one whose `SessionRef` the renderer does not have, and half a
    // session assembled from a segment is a live tile for a project that has none.
    const projectId = data.projectId as ProjectId | undefined;
    if (projectId === undefined) return live;
    const held = live.get(projectId);
    if (held === undefined || held.id !== data.sessionId) return live;
    const credited = data.sessionCreditedSeconds;
    if (typeof credited !== 'number') return live;
    const next = new Map(live);
    next.set(projectId, { ...held, creditedSeconds: credited });
    return next;
  }

  return live;
}

export function useSessions(deps: AppDeps): ReadonlyMap<ProjectId, SessionRef> {
  const [live, setLive] = useState<ReadonlyMap<ProjectId, SessionRef>>(EMPTY);
  const { subscribe } = deps;

  useEffect(
    () =>
      subscribe((event) => {
        setLive((current) => applySessionEvent(current, event));
      }),
    [subscribe],
  );

  return live;
}
