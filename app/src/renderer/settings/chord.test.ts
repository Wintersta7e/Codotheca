import { describe, expect, it } from 'vitest';
import { SYSTEM_MENU_CONSEQUENCE, residentShortcutStatusText } from '../../shared/chord.js';
import { CHORD_PROMPT, bindOutcomeFor, captureChord } from './chord.js';

const key = (
  over: Partial<Parameters<typeof captureChord>[0]>,
): Parameters<typeof captureChord>[0] => ({
  key: 'k',
  code: 'KeyK',
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...over,
});

describe('recording a chord in the drawer', () => {
  it('accepts a modifier plus a key and reports no warning', () => {
    expect(captureChord(key({ ctrlKey: true, shiftKey: true }))).toEqual({
      kind: 'chord',
      chord: 'Control+Shift+K',
      warning: null,
    });
  });

  it('keeps listening while only modifiers are held', () => {
    expect(captureChord(key({ code: 'ControlLeft', key: 'Control', ctrlKey: true }))).toEqual({
      kind: 'incomplete',
    });
    // A bare key binds nothing: §8.6 wants at least one modifier.
    expect(captureChord(key({}))).toEqual({ kind: 'incomplete' });
  });

  it('treats Escape as cancelling the recording, never as a chord to bind', () => {
    expect(captureChord(key({ code: 'Escape', key: 'Escape' }))).toEqual({ kind: 'cancelled' });
    // And still cancels while a modifier is held, rather than proposing Control+Escape.
    expect(captureChord(key({ code: 'Escape', key: 'Escape', ctrlKey: true }))).toEqual({
      kind: 'cancelled',
    });
  });

  it('states the consequence of the system window menu chord rather than refusing it', () => {
    // §8.6: the binder never *proposes* that chord, and states its consequence when it is typed.
    expect(captureChord(key({ code: 'Space', key: ' ', altKey: true }))).toEqual({
      kind: 'chord',
      chord: 'Alt+Space',
      warning: SYSTEM_MENU_CONSEQUENCE,
    });
  });

  it('prompts with the way out, so a recording is never a trap', () => {
    expect(CHORD_PROMPT).toContain('ESC');
  });
});

describe('reading the shell binding back', () => {
  it('an unbound shortcut reads NOT SET', () => {
    expect(residentShortcutStatusText(bindOutcomeFor({ chord: null, registered: false }))).toBe(
      'NOT SET',
    );
    // The core stores the cleared shortcut as the empty string, which is also unbound.
    expect(bindOutcomeFor({ chord: '', registered: false })).toEqual({ kind: 'unbound' });
  });

  it('a bound shortcut reads its chord', () => {
    expect(
      residentShortcutStatusText(bindOutcomeFor({ chord: 'Control+Shift+K', registered: true })),
    ).toBe('Control+Shift+K');
  });

  it('a refused chord reads back which chord and why, and never as bound', () => {
    // Criterion 61: a failed registration is never silent and never renders as bound.
    const outcome = bindOutcomeFor({ chord: 'Control+Shift+K', registered: false });
    expect(outcome.kind).toBe('refused');
    expect(residentShortcutStatusText(outcome)).toBe(
      'NOT SET · Control+Shift+K IS HELD BY ANOTHER APPLICATION',
    );
  });
});
