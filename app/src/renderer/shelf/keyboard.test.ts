import { describe, expect, it } from 'vitest';
import type { ProjectId } from '../../generated/protocol.js';
import type { KeyEventLike } from '../keyboard/contexts.js';
import { resolveKey } from '../keyboard/contexts.js';
import { NEEDS_FOCUSED_PROJECT, shelfKeyIntent } from './keyboard.js';

const key = (over: Partial<KeyEventLike>): KeyEventLike => ({
  key: '',
  code: '',
  altKey: false,
  ctrlKey: false,
  shiftKey: false,
  metaKey: false,
  target: null,
  ...over,
});
const inField = { tagName: 'INPUT' } as const;
const onGrid = { tagName: 'DIV' } as const;
const focused = { peekOpen: false, focusedProjectId: 7 as ProjectId };
const nothingFocused = { peekOpen: false, focusedProjectId: null };

describe('shelfKeyIntent', () => {
  it('claims the crossing binding even from inside the query field', () => {
    // Criterion 32: the palette opens, preventDefault fires, and the keystroke never reaches
    // the field.
    const intent = shelfKeyIntent(
      key({ key: ' ', code: 'Space', altKey: true, target: inField }),
      focused,
    );
    expect(intent).toEqual({ kind: 'claim', action: 'quickSwitch', preventDefault: true });
  });

  it('declines every other key while a text entry has focus', () => {
    // Criterion 55: P is inert while the query field holds focus. A typed p is a p.
    expect(shelfKeyIntent(key({ key: 'p', code: 'KeyP', target: inField }), focused)).toEqual({
      kind: 'decline',
      reason: 'textEntry',
    });
  });

  it('declines Space in the field, so typing a word does not open Peek', () => {
    expect(shelfKeyIntent(key({ key: ' ', code: 'Space', target: inField }), focused)).toEqual({
      kind: 'decline',
      reason: 'textEntry',
    });
  });

  it('declines a text entry the field reaches by contentEditable, not only by tag', () => {
    const editable = { tagName: 'DIV', isContentEditable: true } as const;
    expect(shelfKeyIntent(key({ key: 'p', code: 'KeyP', target: editable }), focused)).toEqual({
      kind: 'decline',
      reason: 'textEntry',
    });
  });

  it('a decline never carries preventDefault, because the field must receive the keystroke', () => {
    const intent = shelfKeyIntent(key({ key: 'p', code: 'KeyP', target: inField }), focused);
    expect(intent).not.toHaveProperty('preventDefault');
  });

  it('reports the decline that 12b’s per-context guard can only report as silence', () => {
    // §11.7's text-entry guard lives inside `resolveKey`, per context (R45), so it answers
    // `null` for every shelf key while an INPUT has focus — the same answer it gives for a key
    // the shelf does not bind at all. This module has to tell the two apart: a declined key is
    // one the caller must let through, and an unbound key is none of the shelf's business.
    const typed = key({ key: 'p', code: 'KeyP', target: inField });
    expect(resolveKey('shelf', typed)).toBeNull();
    expect(shelfKeyIntent(typed, focused)).toEqual({ kind: 'decline', reason: 'textEntry' });
    expect(shelfKeyIntent(key({ key: 'q', code: 'KeyQ', target: inField }), focused)).toBeNull();
  });

  it('claims P on a focused card', () => {
    // `preventDefault` is 12b's table's value, not this module's: a bare letter outside a text
    // entry has no browser default to suppress.
    expect(shelfKeyIntent(key({ key: 'p', code: 'KeyP', target: onGrid }), focused)).toEqual({
      kind: 'claim',
      action: 'shelf.togglePin',
      preventDefault: false,
    });
  });

  it('carries 12b’s preventDefault verbatim rather than deciding it again', () => {
    // One owner per value. If the table ever changes its mind about a key, this module moves
    // with it and no second table has to be found.
    for (const code of ['ArrowDown', 'Space', 'Enter', 'KeyP', 'Escape']) {
      const event = key({ code, target: onGrid });
      const intent = shelfKeyIntent(event, { peekOpen: true, focusedProjectId: 7 as ProjectId });
      expect(intent).not.toBeNull();
      if (intent?.kind === 'claim') {
        expect(intent.preventDefault).toBe(resolveKey('shelf', event)?.preventDefault);
      }
    }
  });

  it('declines an action that needs a card when no card is focused', () => {
    // Enter with nothing focused must not launch, and must not be swallowed either.
    expect(shelfKeyIntent(key({ key: 'Enter', code: 'Enter', target: onGrid }), nothingFocused)) //
      .toEqual({ kind: 'decline', reason: 'noFocusedProject' });
  });

  it('names every card-addressed action, so none is silently actionable with nothing focused', () => {
    for (const action of NEEDS_FOCUSED_PROJECT) expect(action.startsWith('shelf.')).toBe(true);
    expect(NEEDS_FOCUSED_PROJECT).not.toContain('shelf.saveCollection');
    expect(NEEDS_FOCUSED_PROJECT).not.toContain('quickSwitch');
  });

  it('declines Esc when no Peek is open — Esc closes Peek only, never the selection', () => {
    expect(shelfKeyIntent(key({ key: 'Escape', code: 'Escape', target: onGrid }), focused)).toEqual(
      { kind: 'decline', reason: 'noPeekOpen' },
    );
  });

  it('claims Esc when a Peek is open', () => {
    const intent = shelfKeyIntent(key({ key: 'Escape', code: 'Escape', target: onGrid }), {
      peekOpen: true,
      focusedProjectId: 7 as ProjectId,
    });
    expect(intent).toEqual({
      kind: 'claim',
      action: 'shelf.closePeek',
      preventDefault: false,
    });
  });

  it("claims Ctrl+S over Chromium's Save-Page accelerator", () => {
    expect(
      shelfKeyIntent(key({ key: 's', code: 'KeyS', ctrlKey: true, target: onGrid }), focused),
    ).toEqual({ kind: 'claim', action: 'shelf.saveCollection', preventDefault: true });
  });

  it('claims Ctrl+S with nothing focused — a collection is not addressed at a card', () => {
    expect(
      shelfKeyIntent(
        key({ key: 's', code: 'KeyS', ctrlKey: true, target: onGrid }),
        nothingFocused,
      ),
    ).toEqual({ kind: 'claim', action: 'shelf.saveCollection', preventDefault: true });
  });

  it('returns null for a key the shelf does not bind at all', () => {
    expect(shelfKeyIntent(key({ key: 'q', code: 'KeyQ', target: onGrid }), focused)).toBeNull();
  });
});
