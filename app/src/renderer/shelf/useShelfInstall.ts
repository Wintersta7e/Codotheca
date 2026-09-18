/**
 * §24.3d's Install offer, as the shelf holds it.
 *
 * **One preview per project, asked for when a tile demands it.** A tile is mounted only while it
 * is near the viewport, so the demand is the virtualizer's own window and a library of hundreds
 * of not-cloned projects costs the previews that are on screen rather than all of them. Each is
 * asked once and remembered: `install.preview` is read-only and unprivileged, but it is still a
 * round trip.
 *
 * **Without a stored destination there is no offer at all.** `install.preview` takes a `RootId`
 * and §24.3a stores it as `Settings.installRootId`; a shelf tile has nowhere to put the chooser,
 * so where no root has been chosen the card offers nothing and the project page is where the
 * choice is made.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { InstallPreview, ProjectId, RootId } from '../../generated/protocol';
import type { AppDeps } from '../app/deps';

export interface ShelfInstall {
  /** The answers so far. A project absent from the map has no preview yet, so no offer. */
  readonly previews: ReadonlyMap<ProjectId, InstallPreview>;
  /** A mounted tile with no working copy saying it would like one. */
  readonly need: (projectId: ProjectId) => void;
  readonly start: (projectId: ProjectId) => void;
}

export function useShelfInstall(deps: AppDeps): ShelfInstall {
  const [previews, setPreviews] = useState<ReadonlyMap<ProjectId, InstallPreview>>(new Map());
  const [rootId, setRootId] = useState<RootId | null>(null);
  // Asked-for ids, including the ones still in flight: without this a re-render between the
  // request and its answer asks again, and a scroll would ask once per frame.
  const asked = useRef(new Set<ProjectId>());
  const starting = useRef(new Set<ProjectId>());
  /**
   * Every tile that has demanded a preview, whether or not one could be asked for yet.
   *
   * **The first paint is the case this exists for.** `settings.get` is a round trip, and every
   * tile on screen mounts before it answers — a demand dropped because the destination was not
   * known yet would never be repeated, because the tile's effect does not run again. The result
   * was a shelf on which no card was ever offered an install until the user scrolled one out of
   * view and back.
   */
  const wanted = useRef(new Set<ProjectId>());

  useEffect(() => {
    let live = true;
    deps
      .request('settings.get', {})
      .then((settings) => {
        if (live) setRootId(settings.installRootId);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [deps]);

  const need = useCallback(
    (projectId: ProjectId) => {
      wanted.current.add(projectId);
      if (rootId === null || asked.current.has(projectId)) return;
      asked.current.add(projectId);
      deps
        .request('install.preview', { projectId, rootId })
        .then((preview) => {
          setPreviews((held) => new Map(held).set(projectId, preview));
        })
        .catch(() => {
          // No preview is no offer. The tile draws its blueprint and nothing else, which is what
          // it did before this existed.
          asked.current.delete(projectId);
        });
    },
    [deps, rootId],
  );

  // The destination arriving is the other half of `need`: every tile that asked before it was
  // known is served now, once, and a shelf that mounted before the read resolved is not empty.
  useEffect(() => {
    if (rootId === null) return;
    for (const projectId of wanted.current) need(projectId);
  }, [need, rootId]);

  const start = useCallback(
    (projectId: ProjectId) => {
      // §2.4: the start is non-idempotent. The guard holds within the tick as well as across the
      // re-render a state flag would have to wait for.
      if (rootId === null || starting.current.has(projectId)) return;
      starting.current.add(projectId);
      deps
        .installStart(projectId, rootId)
        .then(() => {
          starting.current.delete(projectId);
          // Whatever the answer was — a run id or a refusal — the core's current statement of
          // this destination is the preview, so it is re-read rather than composed here.
          asked.current.delete(projectId);
          need(projectId);
        })
        .catch(() => {
          starting.current.delete(projectId);
        });
    },
    [deps, need, rootId],
  );

  return { previews, need, start };
}
