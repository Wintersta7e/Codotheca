import type { ReactElement } from 'react';
import type { ViewMode } from '../../generated/protocol.js';
import type { DensityStepName } from '../card/geometry.js';
import { densityStep } from '../card/geometry.js';
import { clampDensity, nextDensity } from './viewState.js';

export const DENSITY_KEY_LABEL = 'DENSITY' as const;

/** 12b's step names are `compact | default | roomy`; §8.0a's third visible word is `LARGE`.
 *  The translation lives here and nowhere else. */
export const DENSITY_LABELS: Readonly<Record<DensityStepName, string>> = {
  compact: 'COMPACT',
  default: 'DEFAULT',
  roomy: 'LARGE',
};

function densityWord(density: number): string {
  return DENSITY_LABELS[densityStep(clampDensity(density)).name];
}

export function densityControlName(density: number): string {
  return `Density: ${densityWord(density).toLowerCase()}`;
}

export interface DensityControlProps {
  readonly density: number;
  readonly viewMode: ViewMode;
  readonly showKey: boolean;
  readonly onCycle: (next: number) => void;
}

/** The cycling three-step control in the vacated `LIBRARY` slot. It shows the **word**, never
 *  the pixel count: the shed order drops the key before the value, and a bare `186` in a
 *  narrowed window names nothing a reader can say. */
export function DensityControl(props: DensityControlProps): ReactElement | null {
  // §11.3a's dead-switch rule: density cannot act on the audit table, so there is no control.
  if (props.viewMode === 'list') return null;
  return (
    <button
      type="button"
      data-slot="density"
      className="cdt-shelf-control cdt-shelf-control-cycles"
      aria-label={densityControlName(props.density)}
      onClick={() => {
        props.onCycle(nextDensity(props.density));
      }}
    >
      {props.showKey ? <span className="cdt-shelf-control-key">{DENSITY_KEY_LABEL}</span> : null}
      <span className="cdt-shelf-control-value" aria-live="polite">
        {densityWord(props.density)}
      </span>
    </button>
  );
}
