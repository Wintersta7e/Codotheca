/**
 * §48.7.1 step 5: the last rebuild's report, until the user acknowledges it.
 *
 * Read when the window mounts and again on every `core/snapshot`. The lane turns `ready` on the
 * core's greeting, and a rebuild runs after that greeting, so at `ready` the report may not exist
 * yet; the snapshot answers a subscription the core reads only once its startup is over, by
 * which point any report is written. `null` is *no report*. Acknowledging asks the shell to
 * remove the file and stops raising the notice, which is the notice's only dismissal.
 */
import { useCallback, useEffect, useState } from 'react';

import type { RebuildReport } from '../../shared/rebuildReport.js';
import type { AppDeps } from './deps.js';

export interface RebuildReportState {
  readonly report: RebuildReport | null;
  readonly acknowledge: () => void;
}

export function useRebuildReport(deps: AppDeps): RebuildReportState {
  const [report, setReport] = useState<RebuildReport | null>(null);
  const { rebuildReport, ackRebuildReport, subscribe } = deps;

  useEffect(() => {
    let live = true;
    const read = (): void => {
      void rebuildReport().then(
        (answer) => {
          if (live) setReport(answer);
        },
        () => {
          // Unread stays unread: no notice beats one stating figures nobody read.
        },
      );
    };
    read();
    const off = subscribe((event) => {
      if (event.topic === 'core' && event.event === 'snapshot') read();
    });
    return () => {
      live = false;
      off();
    };
  }, [rebuildReport, subscribe]);

  const acknowledge = useCallback(() => {
    setReport(null);
    void ackRebuildReport().catch(() => undefined);
  }, [ackRebuildReport]);

  return { report, acknowledge };
}
