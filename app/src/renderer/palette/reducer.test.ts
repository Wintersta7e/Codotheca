import { describe, expect, it } from 'vitest';
import type { KeyContext, KeyEventLike } from '../keyboard/contexts.js';
import { resolveKey } from '../keyboard/contexts.js';
import { PALETTE_CLOSED, paletteIntent, paletteReducer } from './reducer.js';
import type { PaletteState } from './reducer.js';

const OPEN: PaletteState = { open: true, query: 'ni', cursor: 2 };

// R12: plan 12b's shape, imported, not restated. It also carries `key` and `target`; the reducer
// reads neither, and the fixture supplies them so the type is satisfied honestly rather than cast.
const key = (over: Partial<KeyEventLike>): KeyEventLike => ({
  key: 'a',
  code: 'KeyA',
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  target: null,
  ...over,
});

describe('paletteReducer', () => {
  // §8.6: it opens empty. Nothing carries over from the last time it was open.
  it('opens empty, at the first row', () => {
    expect(paletteReducer(PALETTE_CLOSED, { type: 'open' })).toEqual({
      open: true,
      query: '',
      cursor: 0,
    });
  });

  it('re-opening an open palette changes nothing, so the entry cannot replay', () => {
    expect(paletteReducer(OPEN, { type: 'open' })).toBe(OPEN);
  });

  it('toggle closes an open palette and opens a closed one', () => {
    expect(paletteReducer(OPEN, { type: 'toggle' })).toEqual(PALETTE_CLOSED);
    expect(paletteReducer(PALETTE_CLOSED, { type: 'toggle' })).toEqual({
      open: true,
      query: '',
      cursor: 0,
    });
  });

  it('typing resets the cursor to the first row', () => {
    expect(paletteReducer(OPEN, { type: 'query', value: 'nig' })).toEqual({
      open: true,
      query: 'nig',
      cursor: 0,
    });
  });

  it('clamps movement at both ends and never wraps', () => {
    const at = (cursor: number, delta: 1 | -1, count: number): PaletteState =>
      paletteReducer({ open: true, query: '', cursor }, { type: 'move', delta, count });
    expect(at(0, -1, 5)).toEqual({ open: true, query: '', cursor: 0 });
    expect(at(4, 1, 5)).toEqual({ open: true, query: '', cursor: 4 });
    expect(at(1, 1, 5)).toEqual({ open: true, query: '', cursor: 2 });
    expect(at(3, -1, 5)).toEqual({ open: true, query: '', cursor: 2 });
    expect(at(0, 1, 0)).toEqual({ open: true, query: '', cursor: 0 });
  });

  it('ignores every action but open and toggle while closed', () => {
    expect(paletteReducer(PALETTE_CLOSED, { type: 'query', value: 'x' })).toBe(PALETTE_CLOSED);
    expect(paletteReducer(PALETTE_CLOSED, { type: 'move', delta: 1, count: 9 })).toBe(
      PALETTE_CLOSED,
    );
    expect(paletteReducer(PALETTE_CLOSED, { type: 'close' })).toBe(PALETTE_CLOSED);
  });

  it('a pointer landing on a row selects it', () => {
    expect(paletteReducer(OPEN, { type: 'point', index: 5 })).toEqual({
      open: true,
      query: 'ni',
      cursor: 5,
    });
  });
});

// R42: the resolution is 12b's and is asserted in 12b's own tests. These drive the **real**
// `resolveKey` and assert only the mapping, so a change to 12b's table fails here instead of
// drifting apart from a second copy of it.
const intent = (context: KeyContext, over: Partial<KeyEventLike>): unknown => {
  const resolved = resolveKey(context, key(over));
  return resolved === null ? null : paletteIntent(resolved.action);
};

describe('paletteIntent', () => {
  it('maps the chord to toggle, from the ambient context and from the open palette', () => {
    // The caller picks the context; `quickSwitch` crosses all of them, which is what the deleted
    // `open: boolean` was standing in for.
    expect(intent('shelf', { code: 'Space', altKey: true })).toEqual({ kind: 'toggle' });
    expect(intent('palette', { code: 'Space', altKey: true })).toEqual({ kind: 'toggle' });
  });

  it('is null when 12b claims nothing', () => {
    expect(intent('shelf', { code: 'Space', altKey: true, ctrlKey: true })).toBeNull();
    expect(intent('shelf', { code: 'Space', altKey: true, metaKey: true })).toBeNull();
  });

  it('maps §11.7’s palette row', () => {
    expect(intent('palette', { code: 'ArrowDown' })).toEqual({ kind: 'move', delta: 1 });
    expect(intent('palette', { code: 'ArrowUp' })).toEqual({ kind: 'move', delta: -1 });
    expect(intent('palette', { key: 'Enter', code: 'Enter' })).toEqual({ kind: 'launch' });
    expect(intent('palette', { key: 'Enter', code: 'Enter', shiftKey: true })).toEqual({
      kind: 'openPage',
    });
    expect(intent('palette', { key: 'Escape', code: 'Escape' })).toEqual({ kind: 'close' });
  });

  // R45: the numpad's Enter is a different `code`, and 12b's table carries it. A palette that
  // launched only on the main Enter would drop numpad launch silently.
  it('maps the numpad’s Enter the same way', () => {
    expect(intent('palette', { key: 'Enter', code: 'NumpadEnter' })).toEqual({ kind: 'launch' });
  });

  // R45: the open palette's query bar is an autoFocus'd <input>, so every key it owns arrives
  // with a text-entry target. The mapping has to survive that, and 12b's per-context guard is
  // what makes it: a target the product actually has, not the `null` a window dispatch gives.
  it('maps the palette row with a text entry focused, which is the only state it has', () => {
    const inField = { target: { tagName: 'INPUT', isContentEditable: false } };
    expect(intent('palette', { ...inField, code: 'ArrowDown' })).toEqual({
      kind: 'move',
      delta: 1,
    });
    expect(intent('palette', { ...inField, key: 'Escape', code: 'Escape' })).toEqual({
      kind: 'close',
    });
  });

  // Each context owns its keys exclusively (§11.7): the palette's row must not fire on the shelf.
  // From the shelf those keys resolve to *shelf* actions, and the palette maps none of them.
  it('maps none of another context’s actions', () => {
    for (const code of ['ArrowDown', 'ArrowUp', 'Enter', 'Escape']) {
      const resolved = resolveKey('shelf', key({ key: code, code }));
      if (resolved === null) throw new Error(`the shelf context must own ${code}`);
      expect(paletteIntent(resolved.action), code).toBeNull();
    }
  });
});
