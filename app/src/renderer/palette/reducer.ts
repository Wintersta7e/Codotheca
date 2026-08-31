// R42: 12b owns the one key table, so this file resolves nothing. It maps 12b's `KeyAction` to
// the intent the reducer takes — a separate type only because `move` carries a `delta` no action
// name can. R12's reason still holds for the event shape; it is simply no longer needed here,
// because no event reaches this module.
import type { KeyAction } from '../keyboard/contexts.js';

export type PaletteState =
  | { readonly open: false }
  | { readonly open: true; readonly query: string; readonly cursor: number };

export const PALETTE_CLOSED: PaletteState = { open: false };

export type PaletteAction =
  | { readonly type: 'open' }
  | { readonly type: 'close' }
  | { readonly type: 'toggle' }
  | { readonly type: 'query'; readonly value: string }
  | { readonly type: 'move'; readonly delta: 1 | -1; readonly count: number }
  | { readonly type: 'point'; readonly index: number };

const OPENED: PaletteState = { open: true, query: '', cursor: 0 };

export function paletteReducer(state: PaletteState, action: PaletteAction): PaletteState {
  switch (action.type) {
    case 'open':
      // §8.6: it opens empty. Re-opening an open palette is inert so the entry never replays.
      return state.open ? state : OPENED;
    case 'close':
      return state.open ? PALETTE_CLOSED : state;
    case 'toggle':
      return state.open ? PALETTE_CLOSED : OPENED;
    case 'query':
      return state.open ? { open: true, query: action.value, cursor: 0 } : state;
    case 'move': {
      if (!state.open) return state;
      const last = Math.max(0, action.count - 1);
      const next = Math.min(last, Math.max(0, state.cursor + action.delta));
      return next === state.cursor ? state : { ...state, cursor: next };
    }
    case 'point':
      if (!state.open || state.cursor === action.index) return state;
      return { ...state, cursor: action.index };
  }
}

export type PaletteKey =
  | { readonly kind: 'toggle' }
  | { readonly kind: 'move'; readonly delta: 1 | -1 }
  | { readonly kind: 'launch' }
  | { readonly kind: 'openPage' }
  | { readonly kind: 'close' };

/**
 * R42: the palette's half of 12b's `CONTEXT_ACTIONS`, plus the one action that crosses every
 * context. `null` for anything else — including another context's actions, which reach here only
 * if a caller resolved in the wrong context. There is no `none` variant: `resolveKey` already
 * returns `null` for a key its context does not own, and a second spelling of absence is the
 * duplication R42 removes.
 */
export function paletteIntent(action: KeyAction): PaletteKey | null {
  switch (action) {
    case 'quickSwitch':
      return { kind: 'toggle' };
    case 'palette.moveDown':
      return { kind: 'move', delta: 1 };
    case 'palette.moveUp':
      return { kind: 'move', delta: -1 };
    case 'palette.launch':
      return { kind: 'launch' };
    case 'palette.openPage':
      return { kind: 'openPage' };
    case 'palette.close':
      return { kind: 'close' };
    default:
      return null;
  }
}
