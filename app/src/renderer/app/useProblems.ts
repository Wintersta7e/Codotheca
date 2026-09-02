/**
 * §11.1's report, read once per finished run.
 *
 * One owner: §8.0's problems notice and §11.1's scan summary are the same figures, and two
 * `problems.list` calls would be two readings of one run — which is how a banner comes to
 * disagree with the panel it opens.
 *
 * `null` is *not read*. It is never an empty report: a refused read that rendered zero problems
 * would be a clean bill of health the app never got.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { Problems, ScanStatus } from '../../generated/protocol.js';
import type { AppDeps } from './deps.js';

export interface ProblemsState {
  readonly problems: Problems | null;
  readonly reload: () => void;
}

export function useProblems(deps: AppDeps, scan: ScanStatus | null): ProblemsState {
  const [problems, setProblems] = useState<Problems | null>(null);
  const { request } = deps;
  const live = useRef(true);

  // A run still walking has no final count, and §11.1's figures are the run's, not a moment's.
  const runId = scan !== null && !scan.running ? scan.runId : null;

  const read = useCallback(() => {
    if (runId === null) return;
    void request('problems.list', { run: runId }).then(
      (answer) => {
        if (live.current) setProblems(answer);
      },
      () => {
        // Unread stays unread. See the module comment.
      },
    );
  }, [request, runId]);

  useEffect(() => {
    live.current = true;
    read();
    return () => {
      live.current = false;
    };
  }, [read]);

  return { problems, reload: read };
}
