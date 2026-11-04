/**
 * §43.2's level block, by `LevelReading.state` (§38). Floored is not a state: it is
 * `figures.floored` inside `partial` or `computed`, and it draws the level with no bar, because
 * the XP sits below the level's start and a bar would read negative.
 *
 * Nothing here draws an unknown as a number: `notComputed` — and a reading that carries no
 * figures, whatever its state says — is U+2014 and `NOT COMPUTED`, with no bar and no track.
 */
import type { LevelReading, LevelState } from '../../generated/protocol';
import {
  LEVEL_LABEL,
  LEVEL_NOT_COMPUTED,
  UNKNOWN,
  XP_LINE_COMPUTED,
  XP_LINE_FLOORED,
  XP_LINE_PARTIAL,
} from './copy';

export interface LevelBlockModel {
  readonly state: LevelState;
  /** The number slot: the level, or U+2014 when nothing has computed it. */
  readonly number: string;
  /** True while the number slot holds U+2014, drawn in `--unknown-ink`. */
  readonly unknown: boolean;
  /** The XP line under §43.2's table. */
  readonly line: string;
  /** The bar's fill, 0…1, or `null` for no bar and no track. */
  readonly bar: number | null;
  /** The number button's accessible name. */
  readonly name: string;
}

function notComputed(state: LevelState): LevelBlockModel {
  return {
    state,
    number: UNKNOWN,
    unknown: true,
    line: LEVEL_NOT_COMPUTED,
    bar: null,
    name: `${LEVEL_LABEL} ${LEVEL_NOT_COMPUTED}`,
  };
}

/** A state the schema added after this table: fail rather than draw it blank. */
function noRendering(state: never): never {
  throw new Error(`the level block has no rendering for ${String(state)}`);
}

/** `xpBar` is `MotivationState.xpBar`, the core's one fraction; `null` draws no bar. */
export function levelBlockModel(level: LevelReading | null, xpBar: number | null): LevelBlockModel {
  if (level === null) return notComputed('notComputed');
  const state = level.state;
  switch (state) {
    case 'notComputed':
      return notComputed(state);
    case 'partial':
    case 'computed': {
      const figures = level.figures;
      if (figures === null) return notComputed(state);
      const number = String(figures.level);
      const name = `${LEVEL_LABEL} ${number}`;
      if (figures.floored) {
        return { state, number, unknown: false, line: XP_LINE_FLOORED, bar: null, name };
      }
      const remaining = figures.nextLevelAt - figures.xp;
      const line =
        state === 'partial'
          ? XP_LINE_PARTIAL(remaining, level.pendingProjects)
          : XP_LINE_COMPUTED(remaining);
      const bar = xpBar === null ? null : Math.min(1, Math.max(0, xpBar));
      return { state, number, unknown: false, line, bar, name };
    }
    default:
      return noRendering(state);
  }
}
