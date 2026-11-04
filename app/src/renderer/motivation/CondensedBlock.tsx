/**
 * §43.2's condensed top-bar block: `LVL <n>`, three ring dots and the short rhythm, drawn in §8.0a's
 * bar only while the band renders and is scrolled out of view — the shelf decides that, and the
 * bar draws it only before its first shed step.
 *
 * It is one real `<button>`. Activating it brings the band back and moves focus to the line; it
 * acknowledges nothing. A waiting line is text after the level, never a dot: an unread dot is an
 * announcement. Being on screen in an active window is §43.5's `bandShown`, as the band's is.
 */
import { type CSSProperties, type ReactElement, useEffect, useRef } from 'react';
import type { MotivationState } from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import type { AppDeps } from '../app/deps';
import { useWindowActive } from '../motion/useWindowActive';
import { CONDENSED_LEVEL_LABEL, CONDENSED_LINE_WAITS, RHYTHM_SHORT } from './copy';
import { levelBlockModel } from './levelBlock';
import { ringsModel, type RingGlyph } from './rings';

export interface CondensedModel {
  /** The level, or U+2014 when nothing has computed it. */
  readonly level: string;
  /** True while `level` holds U+2014, drawn in `--unknown-ink`. */
  readonly unknown: boolean;
  /** The waits text while a line is pending, else `null`. */
  readonly waits: string | null;
  /** One dot per ring in §39's order; none on day one, when no ring is read yet. */
  readonly dots: readonly RingGlyph[];
  /** The short rhythm, or `null` when the core sent no rings. */
  readonly rhythm: string | null;
}

/** `null` under quiet mode: the block is the band's, and quiet mode hides the band. */
export function condensedModel(state: MotivationState): CondensedModel | null {
  if (!state.shown) return null;
  const level = levelBlockModel(state.level, state.xpBar);
  const rings = state.rings === null ? null : ringsModel(state.rings);
  const rhythm = state.rings?.rhythm ?? null;
  return {
    level: level.number,
    unknown: level.unknown,
    waits: state.pendingLine === null ? null : CONDENSED_LINE_WAITS,
    dots: rings === null || rings.waived ? [] : rings.rings,
    rhythm:
      rhythm === null ? null : RHYTHM_SHORT(rhythm.activeDays, rhythm.target, rhythm.bestWeeks),
  };
}

/**
 * Brings the band back and puts focus on the line, or on the level number when no line waits.
 *
 * The band is block 1a, the first thing in the shelf's scroll container, so the container's top
 * is the band in view. The scroll is a cut: nothing in the band moves at rest.
 */
export function revealBand(root: HTMLElement | null): void {
  if (root === null) return;
  root.scrollTop = 0;
  const target =
    root.querySelector<HTMLElement>(`.cdt-band [${DATA_MOTIVATION}="line"]`) ??
    root.querySelector<HTMLElement>('.cdt-band-level-number');
  target?.focus({ preventScroll: true });
}

export interface CondensedBlockProps {
  readonly state: MotivationState;
  readonly onReveal: () => void;
  /** Only for §43.5's `bandShown`; activation sends nothing. */
  readonly request: AppDeps['request'];
}

const quiet = (id: string): Record<string, string> => ({ [DATA_MOTIVATION]: id });

/** A custom property the stylesheet reads; a fraction is a number, never a width string. */
const fractionStyle = (name: string, value: number): CSSProperties => ({ [name]: String(value) });
const arcStyle = (fraction: number): CSSProperties => fractionStyle('--cdt-band-arc', fraction);

export function CondensedBlock(props: CondensedBlockProps): ReactElement | null {
  const { state, onReveal, request } = props;
  const active = useWindowActive();
  const shown = state.shown && active;
  const wasShown = useRef(false);

  // Mounted only while it is in the bar, so mounted and active is on screen in an active window.
  useEffect(() => {
    if (shown && !wasShown.current) {
      void request('motivation.acknowledge', { event: 'bandShown', level: null }).then(
        () => undefined,
        () => undefined,
      );
    }
    wasShown.current = shown;
  }, [shown, request]);

  const model = condensedModel(state);
  if (model === null) return null;

  return (
    <button type="button" data-slot="motivation" className="cdt-condensed" onClick={onReveal}>
      <span className="cdt-condensed-level" {...quiet('level')}>
        <span className="cdt-condensed-level-label">{CONDENSED_LEVEL_LABEL}</span>
        <span
          className="cdt-condensed-level-figure"
          data-unknown={model.unknown ? 'true' : undefined}
        >
          {model.level}
        </span>
      </span>
      {model.waits === null ? null : (
        <span className="cdt-condensed-waits" {...quiet('line')}>
          {model.waits}
        </span>
      )}
      {model.dots.length === 0 ? null : (
        <span className="cdt-condensed-rings" {...quiet('rings')}>
          {model.dots.map((dot) => (
            <span
              key={dot.key}
              className="cdt-condensed-dot"
              role="img"
              aria-label={dot.name}
              data-fill={dot.fill}
              style={arcStyle(dot.fraction)}
            />
          ))}
        </span>
      )}
      {model.rhythm === null ? null : (
        <span className="cdt-condensed-rhythm" {...quiet('rhythm')}>
          {model.rhythm}
        </span>
      )}
    </button>
  );
}
