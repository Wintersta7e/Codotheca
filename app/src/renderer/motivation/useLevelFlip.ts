import { type RefObject, useCallback, useEffect, useRef, useState } from 'react';
import type { MotivationState } from '../../generated/protocol';
import { FLIP_INITIAL, type FlipEvent, type FlipLevels, flipReducer, type FlipState } from './flip';

/**
 * Where the flip's state lives: outside the band, because §43.6 compares a rise with the level
 * the band last painted **in this renderer's lifetime**, and the band unmounts whenever the shelf
 * does. A reload starts a new lifetime, so its first paint flips nothing.
 */
export interface FlipLifetime {
  current: FlipState;
}

const RENDERER_LIFETIME: FlipLifetime = { current: FLIP_INITIAL };

export interface LevelFlip {
  /** The flip on screen now, or `null`. */
  readonly playing: FlipLevels | null;
  /** The incoming digits: their animation ending ends the flip. */
  readonly incoming: RefObject<HTMLSpanElement | null>;
  /** Ends the flip at its end state. */
  readonly end: () => void;
}

/** The level the band draws as a number: none while the reading carries no figures. */
function paintedLevel(state: MotivationState | null): number | null {
  const level = state?.level ?? null;
  if (level === null || level.state === 'notComputed') return null;
  return level.figures?.level ?? null;
}

/**
 * §43.6 wired to the band: each new state is a `changed` and then a paint, and while a rise is
 * armed and the band is on screen the next pointer press, key press or window focus-gain fires
 * it. No listener is attached while nothing is armed or playing, so idle is untouched.
 */
export function useLevelFlip(
  state: MotivationState | null,
  onScreen: boolean,
  lifetime: FlipLifetime = RENDERER_LIFETIME,
): LevelFlip {
  const [flip, setFlip] = useState(lifetime.current);
  const incoming = useRef<HTMLSpanElement>(null);
  const dispatch = useCallback(
    (event: FlipEvent): void => {
      lifetime.current = flipReducer(lifetime.current, event);
      setFlip(lifetime.current);
    },
    [lifetime],
  );
  const end = useCallback((): void => {
    dispatch({ kind: 'ended' });
  }, [dispatch]);

  useEffect(() => {
    const line = state?.pendingLine ?? null;
    dispatch({ kind: 'changed', level: line?.level ?? null, form: line?.form ?? null });
    dispatch({ kind: 'painted', level: paintedLevel(state) });
  }, [state, dispatch]);

  const listening = flip.playing !== null || (flip.armed !== null && onScreen);
  useEffect(() => {
    if (!listening) return undefined;
    const input = (): void => {
      dispatch({ kind: 'input' });
    };
    // Focus events from elements do not bubble to a non-capturing listener; the guard keeps an
    // element's focus out even so, leaving only the window's own focus-gain.
    const focus = (event: FocusEvent): void => {
      if (!(event.target instanceof Node)) input();
    };
    window.addEventListener('pointerdown', input, true);
    window.addEventListener('keydown', input, true);
    window.addEventListener('focus', focus);
    return () => {
      window.removeEventListener('pointerdown', input, true);
      window.removeEventListener('keydown', input, true);
      window.removeEventListener('focus', focus);
    };
  }, [listening, dispatch]);

  // A native listener, not React's `onAnimationEnd`: React picks a vendor-prefixed event name
  // wherever `AnimationEvent` is missing, and would then hear nothing the element dispatches.
  const isPlaying = flip.playing !== null;
  useEffect(() => {
    const element = incoming.current;
    if (!isPlaying || element === null) return undefined;
    const ended = (event: Event): void => {
      if (event.target === element) end();
    };
    element.addEventListener('animationend', ended);
    return () => {
      element.removeEventListener('animationend', ended);
    };
  }, [isPlaying, end]);

  return { playing: flip.playing, incoming, end };
}
