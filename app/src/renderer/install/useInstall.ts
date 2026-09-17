/**
 * §24.4's install lane, as one tile or one hero sees it.
 *
 * **It reads `install.snapshot` on mount as well as subscribing** (R54). A tile scrolled back
 * into view was unmounted while the stream fired, so a hook that only subscribed would render
 * nothing until the next event — or, worse, replay the stream from its start and show a stage the
 * run has long left. The snapshot is the current state; the events are the deltas after it.
 *
 * **The renderer computes no figure.** `done`, `total` and `bytes` are the core's per-phase
 * answers and are rendered as they arrive. There is no aggregate on the wire, so there is nothing
 * here that could produce a percentage even by accident.
 */
import { useEffect, useMemo, useState } from 'react';

import type {
  InstallFailed,
  InstallFinished,
  InstallStage,
  InstallStarted,
  InstallState,
  ProjectId,
} from '../../generated/protocol.js';
import type { AppDeps } from '../app/deps.js';

export interface InstallView {
  /** The run this project is installing under. */
  readonly runId: number;
  /** Every stage observed so far, in arrival order. The pacer decides which are on screen. */
  readonly observed: readonly InstallStage[];
  /** The composed display form, which only the core may build. */
  readonly destinationDisplay: string;
  /** Set once the run ends badly; `null` while it is running or after it settled. */
  readonly failed: InstallFailed | null;
  /** Set once the run settles. */
  readonly finished: InstallFinished | null;
}

/**
 * The install view for one project, or `null` when it is not installing.
 *
 * `null` means *no run for this project*, which is a different claim from an empty view — and it
 * is what lets a card render its ordinary state rather than an install readout with nothing in it.
 */
export function useInstall(deps: AppDeps, projectId: ProjectId): InstallView | null {
  const [started, setStarted] = useState<readonly InstallStarted[]>([]);
  const [stages, setStages] = useState<readonly InstallStage[]>([]);
  const [failed, setFailed] = useState<InstallFailed | null>(null);
  const [finished, setFinished] = useState<InstallFinished | null>(null);

  const { subscribe } = deps;

  useEffect(
    () =>
      subscribe((event) => {
        if (event.topic !== 'install') return;
        if (event.event === 'snapshot') {
          // The snapshot replaces what is held rather than merging into it: it *is* the current
          // state, and merging would keep a stage from a run the core has already forgotten.
          const snapshot = event.data as InstallState;
          setStarted(snapshot.started);
          setStages(snapshot.runs);
          return;
        }
        if (event.event === 'started') {
          const begun = event.data as InstallStarted;
          setStarted((previous) => [...previous.filter((s) => s.runId !== begun.runId), begun]);
          return;
        }
        if (event.event === 'stage') {
          const stage = event.data as InstallStage;
          setStages((previous) => [...previous, stage]);
          return;
        }
        if (event.event === 'failed') {
          setFailed(event.data as InstallFailed);
          return;
        }
        if (event.event === 'finished') {
          setFinished(event.data as InstallFinished);
        }
      }),
    [subscribe],
  );

  return useMemo(() => {
    // `InstallStage` carries no project id — its field set is pinned so no aggregate can be added
    // — so the run this project owns is found through `started`, which is exactly why the
    // snapshot is two lists joined on `runId` rather than one.
    const mine = started.find((s) => s.projectId === projectId);
    if (mine === undefined) return null;
    return {
      runId: mine.runId,
      observed: stages.filter((stage) => stage.runId === mine.runId),
      destinationDisplay: mine.destinationDisplay,
      failed: failed !== null && failed.runId === mine.runId ? failed : null,
      finished: finished !== null && finished.runId === mine.runId ? finished : null,
    };
  }, [started, stages, failed, finished, projectId]);
}
