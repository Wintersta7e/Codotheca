/**
 * §33.4's last four rules: **the decoded hero is the predicate**, and the reply is keyed by
 * scene.
 *
 * - **The layers mount only when the decoded hero's `scene_hash` equals `Weathering.sceneHash`**,
 *   and unmount the moment it does not. A reroll (§7.4) is a new scene and a new anchor set.
 * - **`art_state = 'ready'` is not the predicate.** `art_state` tracks the `card` rendition only
 *   (§7.6), so a hero nobody has demanded is `ready` with **no file**. §7.5's plate fallback draws
 *   no vents, screws or seams, and **dusting geometry the user cannot see is a claim about a
 *   surface that is not there.**
 * - **The reply is keyed and cached by `sceneHash`** — one fetch per scene, not one per closure.
 *   `ProjectDetail` is re-fetched on every debt change; the anchor set moves only when the art
 *   re-renders. Putting the geometry on `ProjectDetail` would ship it on every item close and
 *   stamp a scene-addressed value with a debt observation time.
 *
 * The blueprint rule (§23.5) is the caller's: `renditionFor` already answers which pass is up,
 * and that predicate is **read, never re-derived**. A caller on a blueprint pass hands this hook
 * no hash.
 */
import { useEffect, useRef, useState } from 'react';
import type { ProjectId, SceneHash, Weathering } from '../../generated/protocol';
import { useProjectPageDeps } from '../project/deps';

export function useWeathering(
  projectId: ProjectId | null,
  decodedSceneHash: SceneHash | null,
): Weathering | null {
  const deps = useProjectPageDeps();
  // Keyed by the scene the reply describes, so a second surface asking for the same scene reuses
  // the answer rather than issuing a second request for a value that cannot have changed.
  const cache = useRef(new Map<string, Weathering>());
  const [, setTick] = useState(0);

  useEffect(() => {
    if (projectId === null || decodedSceneHash === null) return undefined;
    if (cache.current.has(decodedSceneHash)) return undefined;
    let live = true;
    deps
      .request('health.weathering', { projectId })
      .then((reply) => {
        if (!live || reply.sceneHash === null) return;
        cache.current.set(reply.sceneHash, reply);
        setTick((n) => n + 1);
      })
      .catch(() => {
        // §7.5: whatever is on screen stays there. A project whose anchors cannot be read keeps
        // its plate rather than growing a layer set nobody resolved.
      });
    return () => {
      live = false;
    };
  }, [deps, projectId, decodedSceneHash]);

  if (decodedSceneHash === null) return null;
  // The reply has to name the scene on screen. A reply for a previous scene is a correct answer
  // to a question about a different card.
  return cache.current.get(decodedSceneHash) ?? null;
}
