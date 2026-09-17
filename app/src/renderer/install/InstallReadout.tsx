/**
 * §24.4's readout: `Receiving objects · 12,400 of 31,882 · 18.4 MB`.
 *
 * **Within a phase the count is monotonic and the denominator is real.** A phase with no
 * denominator renders a bare count — §10.2's shape, unchanged. There is **no aggregate percentage
 * anywhere**, and no field on the wire to build one from.
 *
 * It plays **in place, on the tile and on the hero** — never a modal and never a full-screen
 * flow, because every full-screen flow unmounts the shelf and an install is a background
 * operation the user may walk away from.
 */
import type { ReactElement } from 'react';

import type { InstallStage, InstallStageKind } from '../../generated/protocol.js';
import { pacedStages, stageBytes, stageFigure } from './stageFloor.js';

/** §24.4's stage names as the user reads them. */
const STAGE_LABEL: Readonly<Record<InstallStageKind, string>> = {
  plans: 'Preparing',
  enumerating: 'Enumerating objects',
  receiving: 'Receiving objects',
  assembling: 'Resolving deltas',
  cladding: 'Checking out files',
  settled: 'Settled',
};

export interface InstallReadoutProps {
  readonly observed: readonly InstallStage[];
  readonly elapsedMs: number;
  /** `tile` or `hero`. The same readout at two sizes — never two readouts. */
  readonly surface: 'tile' | 'hero';
}

export function InstallReadout({
  observed,
  elapsedMs,
  surface,
}: InstallReadoutProps): ReactElement | null {
  const shown = pacedStages(observed, elapsedMs);
  if (shown.length === 0) return null;
  const currentKind = shown[shown.length - 1]!;
  // The last observation for the stage on screen — not the last observation overall, which may
  // belong to a stage the pacer has not revealed yet.
  const current = [...observed].reverse().find((stage) => stage.stage === currentKind);
  if (current === undefined) return null;

  const figure = stageFigure(current);
  const bytes = stageBytes(current);
  const parts = [STAGE_LABEL[currentKind], figure, bytes].filter(
    (part): part is string => part !== null,
  );

  return (
    <div
      className="cdt-install-readout"
      data-surface={surface}
      data-stage={currentKind}
      role="status"
    >
      {parts.join(' · ')}
    </div>
  );
}
