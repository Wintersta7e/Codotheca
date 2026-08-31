import type { ReactElement } from 'react';
import type { ConditionSignal } from '../../generated/protocol';
import { conditionDotName } from '../a11y/names';
import { DOT_SIZE_PX, conditionDot } from '../derive/condition';
import { DOT_SURFACE, type CardSurface, bandsFor } from './geometry';

/**
 * §5.4a's dot, on §7.7's band 1. This component renders it and decides nothing: the table lives
 * in plan 09, the size lives there too, and the inset comes from §7.7's band table. What is
 * decided here is that `condition_signal IS NULL` renders **no node** — absence, which is the
 * honest render of *not computed* and the sibling of §7.7a's claim for completion. A dark disc
 * in a grey ring would be a fourth unknown mark one ring-hue from `offline`, meaning the
 * opposite thing.
 */
export interface ConditionDotMarkProps {
  readonly signal: ConditionSignal | null;
  readonly isReference: boolean;
  readonly isArchived: boolean;
  readonly surface: CardSurface;
}

export function ConditionDotMark(props: ConditionDotMarkProps): ReactElement | null {
  const dot = conditionDot({
    signal: props.signal,
    isReference: props.isReference,
    isArchived: props.isArchived,
  });
  if (dot === null) return null;

  const size = DOT_SIZE_PX[DOT_SURFACE[props.surface]];
  const inset = bandsFor(props.surface).dot;
  const name = conditionDotName(props.signal);

  return (
    <>
      <span
        className="cdt-dot"
        data-testid="cdt-dot"
        aria-hidden="true"
        style={{
          width: `${String(size)}px`,
          height: `${String(size)}px`,
          right: `${String(inset.right)}px`,
          top: `${String(inset.top)}px`,
          borderRadius: '50%',
          ...(dot.fill === null ? {} : { background: dot.fill }),
          ...(dot.ring === null ? {} : { border: dot.ring }),
          ...(dot.glow === null ? {} : { boxShadow: dot.glow }),
        }}
      />
      {name === null ? null : <span className="cdt-visually-hidden">{name}</span>}
    </>
  );
}
