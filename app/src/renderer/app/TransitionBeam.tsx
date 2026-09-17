/**
 * §8.5.1's beam: the one part of the gesture that belongs to neither view.
 *
 * It stretches while the shelf is still on screen and contracts while the page is, so an owner on
 * either side would unmount it halfway through its own animation. It renders above both, draws
 * nothing outside `opening` and `closing`, and never takes a pointer event.
 *
 * **The beam is in that project's jewel**, not the app accent. §7.8 records the prototype
 * hardcoding `--sig` for one hover effect and rules it wrong for the same reason: the light that
 * comes up must look like it belongs to the project, and `--sig` is the app talking about itself.
 */
import { useMemo, type CSSProperties, type ReactElement } from 'react';

import type { ProjectRow } from '../../generated/protocol';
import { appearanceFor, fadeFor, seedOf } from '../art/appearance';
import type { TransitionPhase } from '../motion/transition';

export interface TransitionBeamProps {
  readonly phase: TransitionPhase;
  /** The shelf's rows, to resolve the jewel. An id with no row draws no beam. */
  readonly rows: readonly ProjectRow[];
}

export function TransitionBeam({ phase, rows }: TransitionBeamProps): ReactElement | null {
  const active = phase.kind === 'opening' || phase.kind === 'closing' ? phase : null;
  const id = active?.id ?? null;

  const style = useMemo<CSSProperties | null>(() => {
    if (id === null) return null;
    const row = rows.find((candidate) => candidate.id === id);
    // A beam with no row would have to pick a colour, and every available default is the app
    // talking about itself. Drawing nothing is the honest answer.
    if (row === undefined) return null;
    const appearance = appearanceFor(seedOf(row), fadeFor(row), row.primaryLanguage);
    // Only the jewel crosses from here. The softer halo is derived from it in the stylesheet, so
    // the beam has one colour owner and not two that can disagree about the same light.
    return { '--cdt-beam-tint': appearance.jewel } as CSSProperties;
  }, [id, rows]);

  if (active === null || style === null) return null;

  return (
    <div className="cdt-beam-stage" data-gesture={active.kind} style={style} aria-hidden="true">
      <div className="cdt-beam-flare" />
      <div className="cdt-beam-line" />
    </div>
  );
}
