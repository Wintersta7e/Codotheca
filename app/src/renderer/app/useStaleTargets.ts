/**
 * §8.0's priority-4 input: the startup `targets.verify` sweep's stale launch targets.
 *
 * The shell filters the sweep with its `staleTargets` and hands the window the result, pushed
 * when the sweep lands and read once on mount for a window that opened after the push. `null`
 * is *no sweep has reported*, never *nothing is stale*.
 */
import { useEffect, useState } from 'react';

import type { TargetVerification } from '../../generated/protocol.js';
import type { AppDeps } from './deps.js';

export function useStaleTargets(
  deps: Pick<AppDeps, 'onStaleTargets' | 'staleTargetsNow'>,
): readonly TargetVerification[] | null {
  const [rows, setRows] = useState<readonly TargetVerification[] | null>(null);
  const { onStaleTargets, staleTargetsNow } = deps;

  useEffect(() => {
    let live = true;
    let pushed = false;
    const off = onStaleTargets((next) => {
      pushed = true;
      if (live) setRows(next);
    });
    void staleTargetsNow().then(
      (now) => {
        // A push is newer than the answer to a request sent before it.
        if (live && !pushed && now !== null) setRows(now);
      },
      () => {
        // Unread stays unread: no notice beats one about rows nobody read.
      },
    );
    return () => {
      live = false;
      off();
    };
  }, [onStaleTargets, staleTargetsNow]);

  return rows;
}
