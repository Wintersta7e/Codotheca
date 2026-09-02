/**
 * The core lane, as the renderer sees it. One channel, one topic, one registration.
 *
 * `onCoreStatus` hands back no disposer of its own, so it goes through `AppDeps`' fan-out — a
 * hook that re-registered per mount would accumulate listeners for the window's whole life and
 * never say so.
 */
import { useEffect, useMemo, useState } from 'react';

import type { DegradedReason } from '../../generated/protocol.js';
import type { CoreStatus } from '../../shared/coreStatus.js';
import type { StartupFailure } from '../../shared/startupFailure.js';
import type { AppDeps } from './deps.js';

export interface CoreStatusState {
  readonly lane: CoreStatus;
  /**
   * §11.2a's report, and `null` for a lane that failed without one — a spawn failure has no
   * report, and its five sentences belong to the main process, which is the only side that can
   * stat the binary. This hook authors none of them.
   */
  readonly startupFailure: StartupFailure | null;
  readonly logPath: string;
  /** §8.0's priority-2 notice. `null` is *not degraded*, which is not the same as unknown. */
  readonly degraded: DegradedReason | null;
  /**
   * §10.5a's boundary. Library-wide, stated on `core/snapshot` and nowhere else the renderer
   * can see. `null` means first run has not finished, under which nothing is new — never `0`,
   * which would file every project as an arrival.
   */
  readonly firstRunCompletedAt: number | null;
}

const STARTING: CoreStatus = { kind: 'starting' };

export function useCoreStatus(deps: AppDeps): CoreStatusState {
  const [lane, setLane] = useState<CoreStatus>(STARTING);
  const [degraded, setDegraded] = useState<DegradedReason | null>(null);
  const [firstRunCompletedAt, setFirstRunCompletedAt] = useState<number | null>(null);

  const { onCoreStatus, subscribe, logPath } = deps;

  useEffect(() => onCoreStatus(setLane), [onCoreStatus]);

  useEffect(
    () =>
      subscribe((event) => {
        if (event.topic !== 'core') return;
        const data = (event.data ?? {}) as {
          reason?: unknown;
          firstRunCompletedAt?: unknown;
        };
        if (event.event === 'degraded') {
          setDegraded((data.reason ?? null) as DegradedReason | null);
          return;
        }
        if (event.event === 'snapshot') {
          const at = data.firstRunCompletedAt;
          setFirstRunCompletedAt(typeof at === 'number' ? at : null);
        }
      }),
    [subscribe],
  );

  return useMemo(
    () => ({
      lane,
      startupFailure: lane.kind === 'failed' ? (lane.startupFailure ?? null) : null,
      logPath,
      degraded,
      firstRunCompletedAt,
    }),
    [lane, logPath, degraded, firstRunCompletedAt],
  );
}
