/**
 * `scan.status` once, then the `scan` topic.
 *
 * `null` is *the core has not answered*, which `gateDecision` already reads as `'wait'`. A
 * zeroed status assembled here would answer §10's gate before the core had said anything, and
 * an event arriving first cannot fill one in either: a status built from one `progress` frame
 * would be claiming every field the frame does not carry.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { ScanRunId, ScanStatus } from '../../generated/protocol.js';
import type { RendererEvent } from '../../shared/channels.js';
import type { AppDeps } from './deps.js';

interface Payload {
  readonly runId?: unknown;
  readonly generation?: unknown;
  readonly mode?: unknown;
  readonly startedAt?: unknown;
  readonly endedAt?: unknown;
  readonly walkedDirs?: unknown;
  readonly foundRepos?: unknown;
  readonly indexedProjects?: unknown;
  readonly problemCount?: unknown;
  readonly ambiguousLineageCount?: unknown;
}

const num = (value: unknown, fallback: number): number =>
  typeof value === 'number' ? value : fallback;

/** One `scan` event folded into the status. `null` in, `null` out — see the module comment. */
export function applyScanEvent(status: ScanStatus | null, event: RendererEvent): ScanStatus | null {
  if (status === null || event.topic !== 'scan') return status;
  const data = (event.data ?? {}) as Payload;
  switch (event.event) {
    case 'run_started':
      return {
        ...status,
        runId: (data.runId ?? null) as ScanRunId | null,
        running: true,
        cancelled: false,
        generation: num(data.generation, 0),
        mode: (data.mode ?? null) as ScanStatus['mode'],
        startedAt: (data.startedAt ?? null) as ScanStatus['startedAt'],
        endedAt: null,
        walkedDirs: 0,
        foundRepos: 0,
        indexedProjects: 0,
        // A run that has just begun has counted no problems. Null is *not computed*; zero is a
        // claim the run has not made yet.
        problemCount: null,
        ambiguousLineageCount: null,
      };
    case 'progress':
      return {
        ...status,
        walkedDirs: num(data.walkedDirs, status.walkedDirs),
        foundRepos: num(data.foundRepos, status.foundRepos),
        indexedProjects: num(data.indexedProjects, status.indexedProjects),
      };
    case 'finished':
      return {
        ...status,
        running: false,
        endedAt: (data.endedAt ?? null) as ScanStatus['endedAt'],
        walkedDirs: num(data.walkedDirs, status.walkedDirs),
        foundRepos: num(data.foundRepos, status.foundRepos),
        problemCount: num(data.problemCount, 0),
        ambiguousLineageCount: num(data.ambiguousLineageCount, 0),
      };
    case 'cancelled':
      return {
        ...status,
        running: false,
        cancelled: true,
        endedAt: (data.endedAt ?? null) as ScanStatus['endedAt'],
        indexedProjects: num(data.indexedProjects, status.indexedProjects),
      };
    // `repo_found`, `job_done` and `problem` carry no running total, and inventing one here
    // would make two counters for figures `progress` and `finished` already own.
    default:
      return status;
  }
}

export function useScanStatus(deps: AppDeps): ScanStatus | null {
  const [status, setStatus] = useState<ScanStatus | null>(null);
  const { request, subscribe } = deps;
  const live = useRef(true);

  const reload = useCallback(() => {
    void request('scan.status', {}).then(
      (answer) => {
        if (live.current) setStatus(answer);
      },
      () => {
        // Unanswered stays unanswered: the gate waits rather than deciding on a guess.
      },
    );
  }, [request]);

  useEffect(() => {
    live.current = true;
    reload();
    return () => {
      live.current = false;
    };
  }, [reload]);

  useEffect(
    () =>
      subscribe((event) => {
        setStatus((current) => applyScanEvent(current, event));
      }),
    [subscribe],
  );

  return status;
}
