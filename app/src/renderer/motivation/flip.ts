/**
 * §43.6's flip, as a reducer: the band's one motion. A `motivation/changed` whose line is a
 * `rise` above the level the band last painted arms it; the user's next input fires it; any input
 * during it, or its own end, settles it at its end state.
 *
 * Nothing here reads a clock. A timer, data arriving and a first paint never fire it, and a
 * silent rise, `arrival`, `return` or `badge` never arms it. At most one response per pending
 * level: a level that has played does not arm again.
 */
import type { MotivationLineForm } from '../../generated/protocol';

/** The level the old digits show and the level the new digits bring. */
export interface FlipLevels {
  readonly from: number;
  readonly to: number;
}

export interface FlipState {
  /** The last level the band painted as a number in this renderer's lifetime; `null` before. */
  readonly painted: number | null;
  /** A rise waiting for the user's next input. */
  readonly armed: FlipLevels | null;
  /** The flip on screen now. */
  readonly playing: FlipLevels | null;
  /** The last level a flip played for, so a re-sent line does not play it twice. */
  readonly played: number | null;
}

export type FlipEvent =
  /** A `motivation/changed`: the pending line's level and form, both `null` with no line. */
  | {
      readonly kind: 'changed';
      readonly level: number | null;
      readonly form: MotivationLineForm | null;
    }
  /** The band painted this level as a number; `null` paints no number and moves nothing. */
  | { readonly kind: 'painted'; readonly level: number | null }
  /** A pointer press, a key press or the window gaining focus, with the band on screen. */
  | { readonly kind: 'input' }
  /** The flip's last element finished. */
  | { readonly kind: 'ended' };

export const FLIP_INITIAL: FlipState = { painted: null, armed: null, playing: null, played: null };

function changed(
  state: FlipState,
  level: number | null,
  form: MotivationLineForm | null,
): FlipState {
  if (form !== 'rise' || level === null) return { ...state, armed: null };
  // The same rise re-sent keeps what it armed; a higher one carries the unseen start forward.
  if (state.armed !== null) {
    if (level === state.armed.to) return state;
    if (level > state.armed.to) return { ...state, armed: { from: state.armed.from, to: level } };
  }
  // A first paint is never a rise the user saw coming: nothing painted, nothing to flip from.
  if (state.painted === null || level <= state.painted || level === state.played) {
    return { ...state, armed: null };
  }
  return { ...state, armed: { from: state.painted, to: level } };
}

export function flipReducer(state: FlipState, event: FlipEvent): FlipState {
  switch (event.kind) {
    case 'changed':
      return changed(state, event.level, event.form);
    case 'painted':
      return event.level === null || event.level === state.painted
        ? state
        : { ...state, painted: event.level };
    case 'input':
      // §34.6's rule: an input during the flip ends it at its end state.
      if (state.playing !== null) return { ...state, playing: null };
      if (state.armed === null) return state;
      return { ...state, armed: null, playing: state.armed, played: state.armed.to };
    case 'ended':
      return state.playing === null ? state : { ...state, playing: null };
    default:
      return noEvent(event);
  }
}

/** An event the union added after this switch: fail rather than ignore it. */
function noEvent(event: never): never {
  throw new Error(`the flip has no handling for ${JSON.stringify(event)}`);
}
