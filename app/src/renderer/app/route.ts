/**
 * Which of the four top-level surfaces is on screen, as a pure function.
 *
 * The drawer, the scan summary and the palette are **not** routes. They are overlays over
 * whichever of the four is mounted, because closing one must not navigate.
 *
 * `ProjectId` is carried in the route rather than in a second piece of state beside it: two
 * places holding "which page is open" is two places for it to be wrong.
 */
import type { ProjectId, ScanStatus } from '../../generated/protocol.js';
import type { CoreStatus } from '../../shared/coreStatus.js';
import type { StartupFailure } from '../../shared/startupFailure.js';
import type { FailureFact } from '../failure/copy.js';
import { gateDecision } from '../firstrun/FirstRunGate.js';

export type AppRoute =
  | { readonly kind: 'failure'; readonly fact: FailureFact }
  | { readonly kind: 'first-run' }
  | { readonly kind: 'project'; readonly id: ProjectId }
  | { readonly kind: 'shelf' };

export interface RouteInput {
  readonly lane: CoreStatus;
  /** §11.2a's report, as it arrived on the failed status. */
  readonly startupFailure: StartupFailure | null;
  /** `null` is *the core has not answered*, which `gateDecision` reads as `'wait'`. */
  readonly scan: ScanStatus | null;
  readonly hasStoredShelf: boolean;
  readonly openProjectId: ProjectId | null;
}

export function routeFor(input: RouteInput): AppRoute {
  // 1. §11.2a's full-screen idiom, and it outranks everything: the index did not open, so there
  //    is nothing behind it to show — including an open project page, which would be describing
  //    rows the core cannot read.
  //
  //    It needs *both* a failed lane and a report. A report with a healthy lane is yesterday's,
  //    because the core clears the file the moment it opens the index; a failed lane with no
  //    report is a spawn failure, whose five sentences belong to the main process and are not
  //    written — that one is §8.0's priority-1 notice, which `useNotices` raises.
  if (input.lane.kind === 'failed' && input.startupFailure !== null) {
    return { kind: 'failure', fact: input.startupFailure };
  }

  // 2. §10's gate. `'wait'` belongs here too: the gate owns the blank ground it holds while the
  //    core answers, and falling through would flash an empty shelf under the roots screen.
  if (gateDecision(input.scan, input.hasStoredShelf) !== 'shelf') {
    return { kind: 'first-run' };
  }

  if (input.openProjectId !== null) return { kind: 'project', id: input.openProjectId };
  return { kind: 'shelf' };
}
