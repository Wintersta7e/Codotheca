import type { ProjectId } from '../../generated/protocol.js';
import type { KeyAction, KeyContext, KeyEventLike } from '../keyboard/contexts.js';
import { isTextEntry, resolveKey } from '../keyboard/contexts.js';

export const SHELF_CONTEXT: KeyContext = 'shelf';

export interface ShelfKeyState {
  readonly peekOpen: boolean;
  readonly focusedProjectId: ProjectId | null;
}

export type ShelfDeclineReason = 'textEntry' | 'noFocusedProject' | 'noPeekOpen';

export type ShelfIntent =
  | { readonly kind: 'claim'; readonly action: KeyAction; readonly preventDefault: boolean }
  | { readonly kind: 'decline'; readonly reason: ShelfDeclineReason };

/** §11.7: `Alt+Space` is the one binding that crosses contexts, *including* while the shelf's
 *  query field has focus. It is the only action a text entry does not swallow. */
export const CROSSES_CONTEXTS: readonly KeyAction[] = ['quickSwitch'];

/** Every action addressed at a card. With nothing focused there is no object for the verb, so
 *  the shelf declines rather than acting on an implied first row. */
export const NEEDS_FOCUSED_PROJECT: readonly KeyAction[] = [
  'shelf.moveUp',
  'shelf.moveDown',
  'shelf.moveLeft',
  'shelf.moveRight',
  'shelf.play',
  'shelf.openPage',
  'shelf.peek',
  'shelf.togglePin',
];

/** A target that is demonstrably not a text entry, used to ask 12b's table *which action this
 *  chord means* separately from *whether the shelf may act on it now*. It is a shape the
 *  product really has — the grid container — and never `null`, which is a target no real event
 *  carries and the reason R45's palette bug went unseen for a round. */
const BINDING_PROBE_TARGET: KeyEventLike['target'] = { tagName: 'DIV' };

/**
 * `null`  — not a shelf key; the event is none of the shelf's business.
 * decline — a shelf key the shelf will not act on this time. **The caller must not
 *           `preventDefault`**, which is what lets a typed `p` reach the query field.
 * claim   — act, and `preventDefault` exactly as 12b's table says.
 *
 * `resolveKey` applies §11.7's text-entry guard **inside each context** (R45), so for the shelf
 * it answers `null` both for a key typed into the query field and for a key the shelf does not
 * bind. Those are different answers to the caller, so the binding is resolved against a
 * non-text target and the guard is applied here, where the difference can be reported.
 */
export function shelfKeyIntent(event: KeyEventLike, state: ShelfKeyState): ShelfIntent | null {
  const typing = isTextEntry(event.target);
  const resolution = resolveKey(
    SHELF_CONTEXT,
    typing ? { ...event, target: BINDING_PROBE_TARGET } : event,
  );
  if (resolution === null) return null;
  const { action, preventDefault } = resolution;

  if (CROSSES_CONTEXTS.includes(action)) return { kind: 'claim', action, preventDefault };
  if (typing) return { kind: 'decline', reason: 'textEntry' };
  if (action === 'shelf.closePeek' && !state.peekOpen) {
    return { kind: 'decline', reason: 'noPeekOpen' };
  }
  if (NEEDS_FOCUSED_PROJECT.includes(action) && state.focusedProjectId === null) {
    return { kind: 'decline', reason: 'noFocusedProject' };
  }
  return { kind: 'claim', action, preventDefault };
}
