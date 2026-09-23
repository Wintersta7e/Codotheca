/**
 * §34.6 — **playback: the renderer's predicate, evaluated at arrival and never stored.**
 *
 * `detected_in` is the core's record of *which job observed the change*; whether the surge PLAYS
 * is a second question, answered here: a `foreground` delta, for the project whose page is
 * mounted, on a visible document. Conflating the two is the drift the stored column exists to
 * prevent — if both classes animated alike, half the recorded contract would have no consumer.
 *
 * **A `background` delta never animates — not on detection and not on the next open.** A replay
 * would claim a currency it does not have, and the animation is a truth claim about *when* as
 * much as about *whether*. Nothing here is queued: an event that does not play is simply over,
 * and a queue would need a consumed marker on `health_delta` — a column, a migration, a writer
 * and a reader, in the one table that may be rebuilt once.
 *
 * **The rendered state does not ride this event.** The layers move on `projects.upserted`
 * (R121) whether or not a surge plays; the surge is a second, redundant telling of that change.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { DecayLayer, ProjectHealthDelta, ProjectId } from '../../generated/protocol';
import type { RendererEvent } from '../../shared/channels';
import { useProjectPageDeps } from '../project/deps';
import { RESTORATION_SURGE_MS } from './envelope';
import { selectOrigin, type Selection } from './select';
import type { SurgeRequest } from './Surge';

const NONE: Selection = { kind: 'none' };

/** The inputs that end a surge at once: the end state is the state. */
const ANY_INPUT = ['keydown', 'pointerdown', 'wheel'] as const;

function payloadOf(event: RendererEvent): ProjectHealthDelta | null {
  const data = event.data;
  if (typeof data !== 'object' || data === null) return null;
  const candidate = data as Partial<ProjectHealthDelta>;
  if (typeof candidate.id !== 'number' || !Array.isArray(candidate.layers)) return null;
  return candidate as ProjectHealthDelta;
}

/** §34.6's whole predicate, pure: all three conditions, as they stand when the event arrives. */
export function shouldPlay(
  event: RendererEvent,
  projectId: ProjectId,
  visibility: DocumentVisibilityState,
): boolean {
  if (event.topic !== 'projects' || event.event !== 'health_delta') return false;
  if (visibility !== 'visible') return false;
  const delta = payloadOf(event);
  return delta !== null && delta.id === projectId && delta.detectedIn === 'foreground';
}

/**
 * The page's restoration, if one is playing. `lit` is the page's lit count per layer, read at
 * arrival for §34.5's whole-card rule — the event names only the layers that changed.
 *
 * **One envelope at a time.** A delta arriving while a surge plays is not a second envelope and is
 * not queued behind the first; the layers it moved still render their new values off `upserted`.
 */
export function useHealthDelta(
  projectId: ProjectId,
  lit: ReadonlyMap<DecayLayer, number>,
): SurgeRequest {
  const deps = useProjectPageDeps();
  const [selection, setSelection] = useState<Selection>(NONE);
  const litRef = useRef(lit);
  const playing = useRef(false);

  useEffect(() => {
    litRef.current = lit;
  });

  const end = useCallback(() => {
    playing.current = false;
    setSelection(NONE);
  }, []);

  useEffect(() => {
    const unsubscribe = deps.subscribe((event) => {
      if (playing.current) return;
      if (!shouldPlay(event, projectId, document.visibilityState)) return;
      const delta = payloadOf(event);
      if (delta === null) return;
      const next = selectOrigin(delta.layers, litRef.current);
      if (next.kind === 'none') return;
      playing.current = true;
      setSelection(next);
    });
    return () => {
      unsubscribe();
      // A page that closes, or turns to another project, takes its surge with it.
      end();
    };
  }, [deps, projectId, end]);

  // Bounded: the envelope's timer ends it even where no `animationend` ever fires, and any
  // input ends it before that.
  useEffect(() => {
    if (selection.kind === 'none') return undefined;
    const timer = window.setTimeout(end, RESTORATION_SURGE_MS);
    for (const name of ANY_INPUT) window.addEventListener(name, end, true);
    return () => {
      window.clearTimeout(timer);
      for (const name of ANY_INPUT) window.removeEventListener(name, end, true);
    };
  }, [selection, end]);

  return { selection, end };
}
