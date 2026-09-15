/**
 * The resident §8.3 projection: one store, fed by one topic and re-read when a scan run ends.
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
import type { LibraryPresence } from '../shelf/EmptyState.js';
import { ProjectionStore } from '../shelf/projection.js';
import type { ShelfRow } from '../shelf/row.js';
import type { AppDeps } from './deps.js';

export interface LibraryState {
  /**
   * `null` is *the core has not answered*. It is never `[]`, which is *no projects* and is a
   * different sentence — the one §8.3a's empty state is written about.
   */
  readonly rows: readonly ShelfRow[] | null;
  /**
   * The same three states `rows` carries, named — so the surfaces that cannot hold a `null` row
   * list still receive the distinction instead of a boolean that has already thrown it away.
   */
  readonly presence: LibraryPresence;
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

/**
 * Whether this event is a scan run's last word.
 *
 * **The `projects` topic does not announce what a walk writes.** It carries `upserted` for a
 * project something read, `merged`, `flags_changed`, `condition_changed` and `art_ready` — never
 * the scan's own inserts, which land in SQLite and are published by nobody. A run ending is
 * therefore the one moment the resident projection is known to be stale, and re-reading it is
 * what turns a finished scan into a shelf. Without it a first run indexes everything and shows
 * §8.3a's empty state until the app is restarted, which is what a packaged build did.
 *
 * `cancelled` counts: a cancelled run still wrote every project it reached before it stopped.
 */
export function endsAScanRun(event: RendererEvent): boolean {
  return event.topic === 'scan' && (event.event === 'finished' || event.event === 'cancelled');
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
        if (endsAScanRun(event)) reload();
      }),
    [subscribe, store, reload],
  );

  return useMemo(() => {
    const rows = snapshot?.rows ?? null;
    return {
      rows,
      presence: rows === null ? 'uncomputed' : rows.length === 0 ? 'empty' : 'present',
      generation: snapshot?.generation ?? store.generation,
      store,
      reload,
    } satisfies LibraryState;
  }, [snapshot, store, reload]);
}
