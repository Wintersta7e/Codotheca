/**
 * §33.7's `CONDITION` panel: two needles, one divergence line, one 6px dot.
 *
 * | Needle | Value | Fill |
 * |---|---|---|
 * | outer | `condition_signal` — the interaction clock | §5.4a's row for that value |
 * | inner | `condition_material` — the commit clock | §5.4a's row for that value |
 *
 * **Both fills come from `conditionDot`**, §5.4a's one owner in code, which already answers all
 * seven variants plus the `is_reference` and `is_archived` overrides. §33 restates no fill, no
 * ring and no day edge, and the design's band vocabulary (`warm`, `cooling`) stays forbidden —
 * criterion 58, unchanged.
 *
 * **`condition_material IS NULL` draws no inner needle and no divergence line**, never a needle
 * at zero. That is the unknown-as-zero invariant at the one site phase 3 makes expressible.
 *
 * It mounts in `OVERVIEW`, not in `HEALTH`: §30.7 mounts that tab only when the reading is
 * `frozen` or `live`, and these two clocks are phase-1 derived facts that every project has
 * whether or not a health reading exists. Putting a computed fact behind an unrelated predicate
 * would hide it.
 */
import type { CSSProperties, ReactElement } from 'react';
import type { ConditionSignal } from '../../../generated/protocol';
import { conditionDotName } from '../../a11y/names';
import { DOT_SIZE_PX, conditionDot, type ConditionDot } from '../../derive/condition';
import { divergence, needleAngle } from './dial';

export interface ConditionPanelProps {
  /** The interaction clock. `null` — never indexed — draws no outer needle. */
  readonly signal: ConditionSignal | null;
  /** The commit clock. `null` is **never computed** and draws no inner needle. */
  readonly material: ConditionSignal | null;
  readonly isReference: boolean;
  readonly isArchived: boolean;
}

function needleStyle(signal: ConditionSignal, dot: ConditionDot): CSSProperties {
  return {
    transform: `rotate(${String(needleAngle(signal))}deg)`,
    // A `var()` is not a colour to jsdom's parser and an assignment to `backgroundColor` is
    // dropped, so the fill goes on as a custom property the stylesheet reads. The value is
    // `conditionDot`'s and is never restated here.
    '--cdt-needle-fill': dot.fill ?? 'transparent',
    '--cdt-needle-ring': dot.ring ?? 'none',
  } as CSSProperties;
}

export function ConditionPanel(props: ConditionPanelProps): ReactElement {
  const overrides = { isReference: props.isReference, isArchived: props.isArchived };
  const outer = conditionDot({ signal: props.signal, ...overrides });
  const inner =
    props.material === null ? null : conditionDot({ signal: props.material, ...overrides });
  const reading = props.signal === null ? null : divergence(props.signal, props.material);
  const size = DOT_SIZE_PX.conditionLine;

  return (
    <section className="cp-condition" data-testid="cp-condition" aria-label="Condition">
      <h3 className="cp-condition-title">CONDITION</h3>
      <div className="cp-condition-rose" aria-hidden="true">
        {props.signal === null || outer === null ? null : (
          <span
            className="cp-needle"
            data-needle="signal"
            style={needleStyle(props.signal, outer)}
          />
        )}
        {props.material === null || inner === null ? null : (
          <span
            className="cp-needle"
            data-needle="material"
            style={needleStyle(props.material, inner)}
          />
        )}
      </div>
      <p className="cp-condition-line">
        <span
          className="cp-condition-dot"
          data-dot-surface="conditionLine"
          aria-hidden="true"
          style={
            {
              width: `${String(size)}px`,
              height: `${String(size)}px`,
              '--cdt-dot-fill': outer?.fill ?? 'transparent',
              '--cdt-dot-ring': outer?.ring ?? 'none',
            } as CSSProperties
          }
        />
        {/* §11.7: the mark is decoration; the name is the accessible half, and it is
            `conditionDotName`'s — a second spelling here would be a second vocabulary. */}
        <span className="cdt-visually-hidden">{conditionDotName(props.signal)}</span>
        {reading === null ? null : (
          <span className="cp-condition-divergence" data-testid="cp-condition-divergence">
            {reading}
          </span>
        )}
      </p>
    </section>
  );
}
