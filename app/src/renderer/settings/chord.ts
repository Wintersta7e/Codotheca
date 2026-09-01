/**
 * §11.3a group 6's chord recorder. R10 gives plan 15 the shortcut, so everything that decides
 * what a chord *is* — which keys count, which chord takes the system window menu, what a
 * refusal reads back — is imported from `shared/chord.ts` and no line of it is repeated here.
 *
 * What is left is genuinely renderer-shaped: a DOM key event is already structurally a
 * `ChordDescriptor`, `Escape` has to mean *cancel the recording* rather than *bind Escape*, and
 * the shell publishes `ShortcutState` while the status formatter reads `BindOutcome`. Those two
 * are the same fact in two shapes, and one mapping is cheaper than a second copy of the three
 * strings criterion 61 greps for.
 */
import type { BindOutcome } from '../../main/shortcut.js';
import type { ShortcutState } from '../../shared/channels.js';
import { type ChordDescriptor, chordProposalWarning, recordChord } from '../../shared/chord.js';

export const CHORD_PROMPT = 'PRESS A CHORD · ESC CANCELS';

export type ChordCapture =
  | { readonly kind: 'incomplete' }
  | { readonly kind: 'cancelled' }
  | { readonly kind: 'chord'; readonly chord: string; readonly warning: string | null };

/**
 * `Escape` is checked before the chord is assembled, so `Control+Escape` cancels too: the way
 * out of a recorder has to be the same key however it is pressed, or it is not a way out.
 */
export function captureChord(descriptor: ChordDescriptor): ChordCapture {
  if (descriptor.code === 'Escape') return { kind: 'cancelled' };
  const chord = recordChord(descriptor);
  if (chord === null) return { kind: 'incomplete' };
  return { kind: 'chord', chord, warning: chordProposalWarning(chord) };
}

/**
 * The shell publishes `{chord, registered}`; the status formatter reads `BindOutcome`. The core
 * stores a cleared shortcut as the empty string, which is unbound and not a chord named `''`.
 */
export function bindOutcomeFor(state: ShortcutState): BindOutcome {
  if (state.chord === null || state.chord === '') return { kind: 'unbound' };
  return state.registered
    ? { kind: 'bound', chord: state.chord }
    : { kind: 'refused', chord: state.chord };
}
