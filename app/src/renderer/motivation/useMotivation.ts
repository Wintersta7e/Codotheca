/**
 * §43.15's one read: the motivation layer as the renderer holds it, read once and then moved only
 * by the `motivation` topic's `changed` and `snapshot` events — the core's one publisher is the
 * only change signal, so this hook schedules nothing.
 *
 * It is called once, in the composition root, and its value handed down: the band, the condensed
 * block, the reveal's `LEVEL` and Profile all read the same payload rather than one each. `null`
 * is *not read*, and every reader renders it as nothing — never as a zero.
 */
import { useEffect, useState } from 'react';

import type { MotivationState } from '../../generated/protocol';
import type { AppDeps } from '../app/deps';

/** An event's payload, if it has the one field every `MotivationState` carries. */
function asState(data: unknown): MotivationState | null {
  if (typeof data !== 'object' || data === null) return null;
  return typeof (data as { shown?: unknown }).shown === 'boolean'
    ? (data as MotivationState)
    : null;
}

export function useMotivation(
  deps: Pick<AppDeps, 'request' | 'subscribe'>,
): MotivationState | null {
  const [state, setState] = useState<MotivationState | null>(null);
  const { request, subscribe } = deps;

  useEffect(() => {
    let mounted = true;
    // An event is the core's answer at its own instant; a read answered after one arrived may
    // carry the older state, so once any event has landed the read's answer is dropped.
    let heard = false;
    const unsubscribe = subscribe((event) => {
      if (event.topic !== 'motivation') return;
      if (event.event !== 'changed' && event.event !== 'snapshot') return;
      const next = asState(event.data);
      if (next === null) return;
      heard = true;
      setState(next);
    });
    request('motivation.get', {}).then(
      (answer) => {
        if (mounted && !heard) setState(answer);
      },
      // A refused read leaves `null`: the layer is not read, and nothing renders.
      () => undefined,
    );
    return () => {
      mounted = false;
      unsubscribe();
    };
  }, [request, subscribe]);

  return state;
}
