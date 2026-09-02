/**
 * The resident §8.3 projection, one store, fed by one topic.
 *
 * The store is `shelf/ProjectionStore` and the generation is **the store's**. A counter kept
 * beside it is how `orderKey` comes to disagree with the rows it addresses: the page is built
 * against a generation, and two of them means the shelf can address a row set that is gone.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import type {
  ConditionSignal,
  ProjectId,
  ProjectPage,
  ProjectRow,
} from '../../generated/protocol.js';
import type { RendererEvent } from '../../shared/channels.js';
import { ProjectionStore } from '../shelf/projection.js';
import type { ShelfRow } from '../shelf/row.js';
import type { AppDeps } from './deps.js';

export interface LibraryState {
  /**
   * `null` is *the core has not answered*. It is never `[]`, which is *no projects* and is a
   * different sentence — the one §8.3a's empty state is written about.
   */
  readonly rows: readonly ShelfRow[] | null;
  readonly generation: number;
  readonly store: ProjectionStore;
  readonly reload: () => void;
}

interface Payload {
  readonly generation?: unknown;
  readonly rows?: unknown;
  readonly row?: unknown;
  readonly id?: unknown;
  readonly from?: unknown;
  readonly into?: unknown;
  readonly isPinned?: unknown;
  readonly isArchived?: unknown;
  readonly isHidden?: unknown;
  readonly conditionSignal?: unknown;
}

/**
 * One `projects` event applied to the store.
 *
 * `art_ready` is deliberately absent: §7.1a's hold-and-swap belongs to the one card whose
 * rendition changed, and re-publishing the projection here would swap every bitmap on the
 * shelf for it.
 */
export function applyProjectsEvent(store: ProjectionStore, event: RendererEvent): void {
  if (event.topic !== 'projects') return;
  const data = (event.data ?? {}) as Payload;
  switch (event.event) {
    case 'snapshot':
      store.applySnapshot(
        (data.rows ?? []) as readonly ProjectRow[],
        typeof data.generation === 'number' ? data.generation : store.generation,
      );
      return;
    case 'upserted':
      store.upsert(data.row as ProjectRow);
      return;
    case 'merged':
      store.merge(data.from as ProjectId, data.into as ProjectId);
      return;
    case 'flags_changed':
      store.setFlags(data.id as ProjectId, {
        isPinned: data.isPinned === true,
        isArchived: data.isArchived === true,
        isHidden: data.isHidden === true,
      });
      return;
    case 'condition_changed':
      store.setCondition(data.id as ProjectId, (data.conditionSignal ?? null) as ConditionSignal);
      return;
    default:
      return;
  }
}

interface Snapshot {
  readonly rows: readonly ShelfRow[];
  readonly generation: number;
}

export function useLibrary(deps: AppDeps): LibraryState {
  const store = useMemo(() => new ProjectionStore(), []);
  // `null` until the store has been given something. A read that fails leaves it null, which is
  // *uncomputed* — the shelf must not say "no repositories" about a library nothing could read.
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const { request, subscribe } = deps;
  const live = useRef(true);

  // The store is mutable and its identity never changes, so its own listener is what tells
  // React a row set is new. One subscription, taken for the store's whole life here.
  useEffect(
    () =>
      store.subscribe(() => {
        setSnapshot({ rows: store.rows, generation: store.generation });
      }),
    [store],
  );

  const reload = useCallback(() => {
    void request('projects.list', { query: null, sort: null, window: null }).then(
      (result: ProjectPage) => {
        if (!live.current) return;
        store.applySnapshot(result.rows, result.generation);
      },
      () => {
        // Uncomputed, not empty. See `snapshot` above.
      },
    );
  }, [request, store]);

  useEffect(() => {
    live.current = true;
    reload();
    return () => {
      live.current = false;
    };
  }, [reload]);

  useEffect(
    () =>
      subscribe((event) => {
        applyProjectsEvent(store, event);
      }),
    [subscribe, store],
  );

  return useMemo(
    () => ({
      rows: snapshot?.rows ?? null,
      generation: snapshot?.generation ?? store.generation,
      store,
      reload,
    }),
    [snapshot, store, reload],
  );
}
