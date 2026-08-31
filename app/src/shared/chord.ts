/**
 * §8.6's binding model, pure half. It lives in `shared/` because `app/tsconfig.web.json` includes
 * only `src/renderer`, `src/shared` and `src/generated`: the settings surface that records a chord
 * is in the renderer and cannot import `src/main` at all. Two rules shape it:
 *  - the resident shortcut ships unbound, and the accelerator for the Windows system window menu
 *    is never a default. §11.7 adds that it is never a literal in the shell bundle either, so it
 *    is assembled from parts and recognised from parts — never compared against a written string.
 *  - `globalShortcut.register` returns a boolean and does not throw when another application holds
 *    the combination, so its return value is checked and a refusal is surfaced, never swallowed —
 *    which is what `residentShortcutStatusText` renders.
 *
 * `BindOutcome` is a type-only import from the Electron half; the cycle is erased at build time.
 */
import type { BindOutcome } from '../main/shortcut.js';

const MODIFIER_CODES = new Set([
  'AltLeft',
  'AltRight',
  'ControlLeft',
  'ControlRight',
  'MetaLeft',
  'MetaRight',
  'ShiftLeft',
  'ShiftRight',
]);

const ALT = 'Alt';
const SPACE = 'Space';

/** The consequence the binder states when the system-menu chord is typed (§8.6, verbatim). */
export const SYSTEM_MENU_CONSEQUENCE = `EVERY OTHER WINDOW LOSES ITS ${ALT.toUpperCase()}+${SPACE.toUpperCase()} MENU WHILE CODOTHECA RUNS`;

export interface ChordDescriptor {
  readonly key: string;
  readonly code: string;
  readonly altKey: boolean;
  readonly ctrlKey: boolean;
  readonly metaKey: boolean;
  readonly shiftKey: boolean;
}

/** `KeyK` -> `K`, `Digit4` -> `4`, `F12` -> `F12`, `Space` -> `Space`. */
function keyName(code: string): string | null {
  if (MODIFIER_CODES.has(code)) return null;
  if (code.startsWith('Key')) return code.slice(3);
  if (code.startsWith('Digit')) return code.slice(5);
  if (code.startsWith('Numpad')) return `num${code.slice(6).toLowerCase()}`;
  return code;
}

/** A chord is at least one modifier plus exactly one non-modifier key. */
export function recordChord(descriptor: ChordDescriptor): string | null {
  const key = keyName(descriptor.code);
  if (key === null) return null;
  const parts: string[] = [];
  if (descriptor.ctrlKey) parts.push('Control');
  if (descriptor.altKey) parts.push(ALT);
  if (descriptor.shiftKey) parts.push('Shift');
  if (descriptor.metaKey) parts.push('Super');
  if (parts.length === 0) return null;
  parts.push(key);
  return parts.join('+');
}

/** True for the one chord that is the Windows system window menu, derived rather than spelled. */
export function isSystemWindowMenuChord(chord: string): boolean {
  const parts = chord.split('+');
  return parts.length === 2 && parts[0] === ALT && parts[1] === SPACE;
}

/** §8.6: the binder never proposes that chord, and states its consequence when it is typed. */
export function chordProposalWarning(chord: string): string | null {
  return isSystemWindowMenuChord(chord) ? SYSTEM_MENU_CONSEQUENCE : null;
}

/** §11.3a's row: empty reads `NOT SET`, and a refusal reads back which chord and why. */
export function residentShortcutStatusText(outcome: BindOutcome): string {
  switch (outcome.kind) {
    case 'unbound':
      return 'NOT SET';
    case 'bound':
      return outcome.chord;
    case 'refused':
      return `NOT SET · ${outcome.chord} IS HELD BY ANOTHER APPLICATION`;
  }
}
