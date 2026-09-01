import type { ScanStatus } from '../../generated/protocol';

/** Which of §10's beats is on screen. `shelf` means none — first run is over. */
export type FirstRunPhase = 'roots' | 'scanning' | 'reveal' | 'turn' | 'shelf';

export interface FirstRunState {
  readonly phase: FirstRunPhase;
  /** §10.1b's consent row 1. Ticked by default; unticking is honoured. */
  readonly consented: boolean;
  /** When the walk completed, so the settle can be held before the reveal takes the screen. */
  readonly walkFinishedAt: number | null;
}

export type FirstRunAction =
  | { readonly kind: 'consent'; readonly granted: boolean }
  | { readonly kind: 'dig' }
  | { readonly kind: 'skip_ahead' }
  | { readonly kind: 'walk_finished'; readonly at: number }
  | { readonly kind: 'tick'; readonly at: number }
  | { readonly kind: 'nothing_found' }
  | { readonly kind: 'go_on' }
  | { readonly kind: 'show_me' }
  | { readonly kind: 'not_now' };

export const INITIAL_FIRST_RUN: FirstRunState = {
  phase: 'roots',
  // Ticked by default is a disclosure, not a question: nothing here asks the user to choose,
  // row 1 asks them to confirm (§10.1b).
  consented: true,
  walkFinishedAt: null,
};

/** §10.3a: exactly one settle at walk completion, then this long before the reveal. */
export const SETTLE_HOLD_MS = 700;

/** §10.3: cards arrive in batches on a fixed cadence, never one per discovery. */
export const ARRIVAL_BATCH_MS = 600;

/**
 * §10.5: the reveal never replays.
 *
 * The gate is a scan run rather than `first_run_completed_at`, which §11.3a stamps at the
 * residency answer — later than the turn. Gating on the stamp would replay the whole reveal
 * for anyone who quit in between.
 */
export function shouldRunFirstRun(status: Pick<ScanStatus, 'runId'>): boolean {
  return status.runId === null;
}

export function firstRunReducer(state: FirstRunState, action: FirstRunAction): FirstRunState {
  switch (action.kind) {
    case 'consent':
      return state.phase === 'roots' ? { ...state, consented: action.granted } : state;
    case 'dig':
      // Unticking row 1 leaves DIG inert: there is nothing to index without it.
      return state.phase === 'roots' && state.consented ? { ...state, phase: 'scanning' } : state;
    case 'skip_ahead':
      return state.phase === 'scanning' ? { ...state, phase: 'reveal' } : state;
    case 'walk_finished':
      return state.phase === 'scanning' ? { ...state, walkFinishedAt: action.at } : state;
    case 'tick': {
      if (state.phase !== 'scanning' || state.walkFinishedAt === null) return state;
      return action.at - state.walkFinishedAt >= SETTLE_HOLD_MS
        ? { ...state, phase: 'reveal' }
        : state;
    }
    case 'nothing_found':
      // §10.4a: a shelf with no projects never reaches the reveal and gets §11.1 instead.
      return state.phase === 'shelf' ? state : { ...state, phase: 'shelf' };
    case 'go_on':
      return state.phase === 'reveal' ? { ...state, phase: 'turn' } : state;
    case 'show_me':
    case 'not_now':
      return state.phase === 'turn' ? { ...state, phase: 'shelf' } : state;
    default:
      return state;
  }
}
