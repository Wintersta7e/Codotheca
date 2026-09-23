/**
 * §34.1 and §34.5 — **the restoration surge: one DOM element on one opened hero.**
 *
 * A light front expanding from the place the fix happened, mounted inside `.cdt-plate` as a
 * sibling of `.cdt-art`. It needs no canvas, no WebGL context and no compositor, and it never
 * plays on a grid: only the hero hands `CardPlate` a surge. The layers' own change of value is
 * §33's rendering and happens whether or not a surge plays — **the surge neither drives nor gates
 * it**, so the end state it resolves to is exactly the page with no surge at all.
 *
 * **Four outcomes, and no two render alike.** No decrease mounts nothing; every lit layer reaching
 * zero lights the whole card; a layer whose anchor list is empty or has not arrived gets the wipe
 * — the MAJORITY case, since only `cracks` is guaranteed an anchor (A1b) — and a layer with an
 * anchor gets the front, originating there. `card.css` gives each its own animation.
 *
 * This module positions the surge and carries no coordinate of its own: the origin arrives as a
 * fraction and is written as a percentage, multiplied by no pixel constant.
 */
import type { CSSProperties, ReactElement } from 'react';
import type { Origin } from './origin';
import type { Selection } from './select';

/** What the page hands the hero: the selection at arrival, and how to end it. */
export interface SurgeRequest {
  readonly selection: Selection;
  readonly end: () => void;
}

export interface SurgeProps {
  readonly selection: Selection;
  /** `null` is *no origin* — the wipe — never a point at the card's corner. */
  readonly origin: Origin | null;
  readonly onEnd: () => void;
}

export function Surge({ selection, origin, onEnd }: SurgeProps): ReactElement | null {
  if (selection.kind === 'none') return null;
  if (selection.kind === 'whole') {
    return (
      <span className="cdt-surge" data-surge="whole" aria-hidden="true" onAnimationEnd={onEnd} />
    );
  }
  if (origin === null) {
    return (
      <span
        className="cdt-surge"
        data-surge="wipe"
        data-layer={selection.layer}
        aria-hidden="true"
        onAnimationEnd={onEnd}
      />
    );
  }
  const style = {
    '--cdt-surge-x': `${String(origin.fx * 100)}%`,
    '--cdt-surge-y': `${String(origin.fy * 100)}%`,
  } as CSSProperties;
  return (
    <span
      className="cdt-surge"
      data-surge="point"
      data-layer={selection.layer}
      aria-hidden="true"
      style={style}
      onAnimationEnd={onEnd}
    />
  );
}
