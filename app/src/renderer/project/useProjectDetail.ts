/**
 * `projects.get` plus the three `projects` events that can invalidate it while the page is
 * open. `art_ready` deliberately does not reload the detail: §7.1a's swap holds the decoded
 * bitmap until the new one has decoded, and a full reload would re-render the page around it.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { ErrorCode, ProjectDetail, ProjectId, SceneHash } from '../../generated/protocol';
import type { RendererEvent } from '../../shared/channels';
import { CommandError, useProjectPageDeps } from './deps';

export type DetailState =
  | { kind: 'loading' }
  | { kind: 'ready'; detail: ProjectDetail }
  | { kind: 'failed'; code: ErrorCode };

export interface DetailHandle {
  state: DetailState;
  heroHash: SceneHash | null;
  redirectedTo: ProjectId | null;
  reload: () => void;
}

const RELOADING_EVENTS = new Set(['upserted', 'condition_changed', 'flags_changed']);

function fields(data: unknown): Record<string, unknown> | null {
  if (typeof data !== 'object' || data === null) return null;
  return data as Record<string, unknown>;
}

function numberField(data: unknown, key: string): number | null {
  if (typeof data !== 'object' || data === null) return null;
  const value = (data as Record<string, unknown>)[key];
  return typeof value === 'number' ? value : null;
}

/**
 * The three payloads do not agree on where the id sits, and they do not have to: `upserted`
 * carries a whole row so the shelf can repaint a tile without a round trip, while the other two
 * carry the changed fields alone. Reading `data.id` on all three would silently ignore every
 * `upserted`, and an open page would stop following its own note edits.
 */
function subjectOf(event: RendererEvent): number | null {
  if (event.event !== 'upserted') return numberField(event.data, 'id');
  const data = fields(event.data);
  if (data === null) return null;
  return numberField(data['row'], 'id');
}

export function shouldReload(event: RendererEvent, projectId: ProjectId): boolean {
  if (event.topic !== 'projects' || !RELOADING_EVENTS.has(event.event)) return false;
  return subjectOf(event) === (projectId as unknown as number);
}

export function redirectTarget(event: RendererEvent, projectId: ProjectId): ProjectId | null {
  if (event.topic !== 'projects' || event.event !== 'merged') return null;
  if (numberField(event.data, 'from') !== (projectId as unknown as number)) return null;
  const into = numberField(event.data, 'into');
  return into === null ? null : (into as unknown as ProjectId);
}

export function heroHashFrom(event: RendererEvent, projectId: ProjectId): SceneHash | null {
  if (event.topic !== 'projects' || event.event !== 'art_ready') return null;
  if (numberField(event.data, 'projectId') !== (projectId as unknown as number)) return null;
  const data = fields(event.data);
  if (data?.['rendition'] !== 'hero') return null;
  const hash = data['sceneHash'];
  return typeof hash === 'string' ? (hash as SceneHash) : null;
}

export function useProjectDetail(projectId: ProjectId): DetailHandle {
  const deps = useProjectPageDeps();
  const [state, setState] = useState<DetailState>({ kind: 'loading' });
  const [heroHash, setHeroHash] = useState<SceneHash | null>(null);
  const [redirectedTo, setRedirectedTo] = useState<ProjectId | null>(null);
  const generation = useRef(0);

  const reload = useCallback(() => {
    const mine = ++generation.current;
    deps
      .request('projects.get', { id: projectId })
      .then((detail) => {
        if (generation.current !== mine) return;
        setState({ kind: 'ready', detail });
        setHeroHash(detail.row.artSceneHash);
      })
      .catch((err: unknown) => {
        if (generation.current !== mine) return;
        setState({ kind: 'failed', code: err instanceof CommandError ? err.code : 'INTERNAL' });
      });
  }, [deps, projectId]);

  useEffect(() => {
    setState({ kind: 'loading' });
    setRedirectedTo(null);
    reload();
  }, [reload]);

  useEffect(
    () =>
      deps.subscribe((event) => {
        if (shouldReload(event, projectId)) reload();
        const target = redirectTarget(event, projectId);
        if (target !== null) setRedirectedTo(target);
        const hash = heroHashFrom(event, projectId);
        if (hash !== null) setHeroHash(hash);
      }),
    [deps, projectId, reload],
  );

  return { state, heroHash, redirectedTo, reload };
}
