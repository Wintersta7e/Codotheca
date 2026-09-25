/**
 * §24.8's verdict, as the project page holds it.
 *
 * **The verdict is requested when the affordance opens, and at no other time.**
 * `locations.uninstallPreflight` reads every remote over the network to verify it rather than
 * believe it (§47's verifying read), so it is not free and `protocol.json` says outright it is
 * *not callable on hover*. There is no prefetch here, no hover handler and no effect that runs on mount: the
 * only thing that starts a pre-flight is a press.
 *
 * **The renderer confirms; it never decides.** Nothing in this file composes a disposition, adds
 * a blocker or downgrades one. A pre-flight that does not answer leaves the offer closed — the
 * page states nothing it cannot know, which is the shape `LocationsPanel` already uses for a
 * privileged reply it did not get.
 *
 * **A refusal re-reads rather than explains.** `UninstallReply.refused` carries the core's
 * diagnostic `message`, and §2.4 forbids showing one raw. The core refused because it recomputed
 * a verdict that was no longer `safe`, so the honest answer is the fresh verdict: the offer goes
 * back to checking and asks again.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { LocationId, UninstallVerdict } from '../../../generated/protocol';
import { useProjectPageDeps } from '../deps';

/**
 * What the rail is handed.
 *
 * `undefined` is *this rail is not offering it*, `null` is *the pre-flight is in flight*, and a
 * verdict is a verdict. The three are distinct on purpose: a control that renders enabled and
 * then takes itself away is the shape §24.8 refuses.
 */
export interface UninstallOffer {
  readonly verdict: UninstallVerdict | null | undefined;
  /** The affordance opening. This, and only this, starts a pre-flight. */
  readonly open: () => void;
  /** The press on a `safe` verdict. The core re-verifies and may still refuse. */
  readonly remove: () => void;
}

export function useUninstallOffer(
  locationId: LocationId | null,
  onChanged: () => void,
): UninstallOffer {
  const deps = useProjectPageDeps();
  const [verdict, setVerdict] = useState<UninstallVerdict | null | undefined>(undefined);
  // A reply that arrives after the page moved to another location belongs to nobody. The
  // generation is bumped by every open and by every location change, and a stale reply is
  // dropped rather than rendered against the wrong copy.
  const generation = useRef(0);
  const inFlight = useRef(false);

  useEffect(() => {
    generation.current += 1;
    inFlight.current = false;
    setVerdict(undefined);
  }, [locationId]);

  const check = useCallback(() => {
    if (locationId === null) return;
    generation.current += 1;
    const mine = generation.current;
    setVerdict(null);
    deps
      .request('locations.uninstallPreflight', { locationId })
      .then((fresh) => {
        if (generation.current === mine) setVerdict(fresh);
      })
      .catch(() => {
        // No verdict means no offer. §11.4's failure window owns the copy for a core that is
        // not answering; this affordance says nothing it cannot know, and a second press asks
        // again.
        if (generation.current === mine) setVerdict(undefined);
      });
  }, [deps, locationId]);

  const remove = useCallback(() => {
    // §2.4: the removal is non-idempotent, so the guard is a ref and holds within the tick as
    // well as across the re-render a state flag would have to wait for.
    if (locationId === null || inFlight.current) return;
    inFlight.current = true;
    const mine = generation.current;
    deps
      .uninstall(locationId)
      .then((reply) => {
        inFlight.current = false;
        if (generation.current !== mine) return;
        if (reply.kind === 'uninstalled') {
          setVerdict(undefined);
          onChanged();
          return;
        }
        if (reply.kind === 'refused') {
          // The core recomputed and the answer moved. Show what it is now, not what it was.
          check();
          return;
        }
        setVerdict(undefined);
      })
      .catch(() => {
        inFlight.current = false;
        if (generation.current === mine) setVerdict(undefined);
      });
  }, [check, deps, locationId, onChanged]);

  return { verdict, open: check, remove };
}
