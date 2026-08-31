import { describe, expect, it } from 'vitest';
import {
  CONTEXT_ACTIONS,
  type KeyContext,
  type KeyEventLike,
  QUICK_SWITCH_CHORD,
  isTextEntry,
  resolveKey,
} from './contexts';

const CONTEXTS: readonly KeyContext[] = ['shelf', 'projectPage', 'palette', 'settings'];

const ev = (over: Partial<KeyEventLike> = {}): KeyEventLike => ({
  key: 'a',
  code: 'KeyA',
  altKey: false,
  ctrlKey: false,
  shiftKey: false,
  metaKey: false,
  target: { tagName: 'DIV' },
  ...over,
});

const arrow = (key: string): KeyEventLike => ev({ key, code: key });

describe('the shelf owns its keys', () => {
  it('moves focus on the four arrows', () => {
    expect(resolveKey('shelf', arrow('ArrowUp'))?.action).toBe('shelf.moveUp');
    expect(resolveKey('shelf', arrow('ArrowDown'))?.action).toBe('shelf.moveDown');
    expect(resolveKey('shelf', arrow('ArrowLeft'))?.action).toBe('shelf.moveLeft');
    expect(resolveKey('shelf', arrow('ArrowRight'))?.action).toBe('shelf.moveRight');
  });

  it('plays on Enter and opens the page on Shift+Enter', () => {
    expect(resolveKey('shelf', ev({ key: 'Enter', code: 'Enter' }))?.action).toBe('shelf.play');
    expect(resolveKey('shelf', ev({ key: 'Enter', code: 'Enter', shiftKey: true }))?.action).toBe(
      'shelf.openPage',
    );
  });

  it('peeks on Space and closes it on Esc, and Space is preventDefaulted', () => {
    const peek = resolveKey('shelf', ev({ key: ' ', code: 'Space' }));
    expect(peek?.action).toBe('shelf.peek');
    expect(peek?.preventDefault).toBe(true);
    expect(resolveKey('shelf', ev({ key: 'Escape', code: 'Escape' }))?.action).toBe(
      'shelf.closePeek',
    );
  });

  it('toggles the pin on a bare P, either case', () => {
    expect(resolveKey('shelf', ev({ key: 'p', code: 'KeyP' }))?.action).toBe('shelf.togglePin');
    expect(resolveKey('shelf', ev({ key: 'P', code: 'KeyP', shiftKey: true }))?.action).toBe(
      'shelf.togglePin',
    );
  });

  it('saves the query on Ctrl+S and takes the accelerator away from Chromium', () => {
    const saved = resolveKey('shelf', ev({ key: 's', code: 'KeyS', ctrlKey: true }));
    expect(saved?.action).toBe('shelf.saveCollection');
    expect(saved?.preventDefault).toBe(true);
  });
});

describe('the shelf handler must not run on the project page', () => {
  it('answers the grid keys with null there', () => {
    for (const key of ['ArrowUp', 'ArrowDown']) {
      expect(resolveKey('projectPage', arrow(key))).toBeNull();
    }
    expect(resolveKey('projectPage', ev({ key: 'Enter', code: 'Enter' }))).toBeNull();
    expect(resolveKey('projectPage', ev({ key: ' ', code: 'Space' }))).toBeNull();
    expect(resolveKey('projectPage', ev({ key: 'p', code: 'KeyP' }))).toBeNull();
    expect(resolveKey('projectPage', ev({ key: 's', code: 'KeyS', ctrlKey: true }))).toBeNull();
  });

  it('reads the horizontal arrows as tabs and Esc as back', () => {
    expect(resolveKey('projectPage', arrow('ArrowLeft'))?.action).toBe('page.prevTab');
    expect(resolveKey('projectPage', arrow('ArrowRight'))?.action).toBe('page.nextTab');
    expect(resolveKey('projectPage', ev({ key: 'Escape', code: 'Escape' }))?.action).toBe(
      'page.back',
    );
  });
});

describe('the palette and settings own theirs', () => {
  it('moves, launches, opens and closes in the palette', () => {
    expect(resolveKey('palette', arrow('ArrowUp'))?.action).toBe('palette.moveUp');
    expect(resolveKey('palette', arrow('ArrowDown'))?.action).toBe('palette.moveDown');
    expect(resolveKey('palette', ev({ key: 'Enter', code: 'Enter' }))?.action).toBe(
      'palette.launch',
    );
    expect(resolveKey('palette', ev({ key: 'Enter', code: 'Enter', shiftKey: true }))?.action).toBe(
      'palette.openPage',
    );
    expect(resolveKey('palette', ev({ key: 'Escape', code: 'Escape' }))?.action).toBe(
      'palette.close',
    );
  });

  // [R45] The test whose absence let the palette ship dead. Every other test here dispatches
  // with no target, and `isTextEntry(null)` is false — so a global text-entry guard verified
  // green while the real product, whose palette query bar is an autoFocus'd
  // `<input role="combobox">`, had no working keys at all. Dispatch with the target the product
  // actually has.
  it('keeps every palette key alive while its query bar has focus', () => {
    const inField = { target: { tagName: 'INPUT' } };
    expect(resolveKey('palette', { ...arrow('ArrowUp'), ...inField })?.action).toBe(
      'palette.moveUp',
    );
    expect(resolveKey('palette', { ...arrow('ArrowDown'), ...inField })?.action).toBe(
      'palette.moveDown',
    );
    expect(resolveKey('palette', ev({ key: 'Enter', code: 'Enter', ...inField }))?.action).toBe(
      'palette.launch',
    );
    expect(
      resolveKey('palette', ev({ key: 'Enter', code: 'NumpadEnter', ...inField }))?.action,
    ).toBe('palette.launch');
    expect(resolveKey('palette', ev({ key: 'Escape', code: 'Escape', ...inField }))?.action).toBe(
      'palette.close',
    );
  });

  it('still keeps the other contexts inert inside a text entry', () => {
    const inField = { target: { tagName: 'INPUT' } };
    expect(resolveKey('shelf', { ...arrow('ArrowUp'), ...inField })).toBeNull();
    expect(resolveKey('projectPage', { ...arrow('ArrowLeft'), ...inField })).toBeNull();
    expect(resolveKey('settings', ev({ key: 'Escape', code: 'Escape', ...inField }))).toBeNull();
    // Alt+Space crosses every context, including a focused field.
    expect(
      resolveKey('shelf', ev({ key: ' ', code: 'Space', altKey: true, ...inField }))?.action,
    ).toBe('quickSwitch');
  });

  it('gives the palette no horizontal arrows and settings nothing but Esc', () => {
    expect(resolveKey('palette', arrow('ArrowLeft'))).toBeNull();
    expect(resolveKey('palette', arrow('ArrowRight'))).toBeNull();
    expect(resolveKey('settings', ev({ key: 'Escape', code: 'Escape' }))?.action).toBe(
      'settings.close',
    );
    for (const key of ['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Enter']) {
      expect(resolveKey('settings', arrow(key))).toBeNull();
    }
  });
});

describe('no action leaks between contexts', () => {
  it('resolves only to actions its own context declares', () => {
    const probes: KeyEventLike[] = [
      ...['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Enter', 'Escape'].map(arrow),
      ev({ key: 'Enter', code: 'Enter', shiftKey: true }),
      ev({ key: ' ', code: 'Space' }),
      ev({ key: 'p', code: 'KeyP' }),
      ev({ key: 's', code: 'KeyS', ctrlKey: true }),
      ev({ key: ' ', code: 'Space', altKey: true }),
    ];
    for (const context of CONTEXTS) {
      for (const probe of probes) {
        const resolved = resolveKey(context, probe);
        if (resolved === null) continue;
        expect(CONTEXT_ACTIONS[context]).toContain(resolved.action);
      }
    }
  });
});

describe('Alt+Space crosses every context, including a focused field', () => {
  it('resolves in all four contexts and preventDefaults', () => {
    for (const context of CONTEXTS) {
      const resolved = resolveKey(context, ev({ key: ' ', code: 'Space', altKey: true }));
      expect(resolved?.action).toBe('quickSwitch');
      expect(resolved?.preventDefault).toBe(true);
    }
  });

  it('resolves while an input holds focus, and it is the only thing that does', () => {
    const inField = { tagName: 'INPUT' };
    expect(
      resolveKey('shelf', ev({ key: ' ', code: 'Space', altKey: true, target: inField }))?.action,
    ).toBe('quickSwitch');
    expect(resolveKey('shelf', ev({ key: 'p', code: 'KeyP', target: inField }))).toBeNull();
    expect(resolveKey('shelf', ev({ key: ' ', code: 'Space', target: inField }))).toBeNull();
    expect(resolveKey('shelf', arrow('ArrowDown'))).not.toBeNull();
  });

  it('is detected by code, not by key, so a layout cannot break it', () => {
    expect(
      resolveKey('shelf', ev({ key: 'Unidentified', code: 'Space', altKey: true }))?.action,
    ).toBe('quickSwitch');
    expect(QUICK_SWITCH_CHORD).toBe('Alt+Space');
  });
});

describe('the text-entry guard', () => {
  it('is the prototype guard plus contenteditable', () => {
    expect(isTextEntry({ tagName: 'INPUT' })).toBe(true);
    expect(isTextEntry({ tagName: 'TEXTAREA' })).toBe(true);
    expect(isTextEntry({ tagName: 'DIV', isContentEditable: true })).toBe(true);
    expect(isTextEntry({ tagName: 'DIV' })).toBe(false);
    expect(isTextEntry(null)).toBe(false);
  });
});

describe('modifiers phase 1 does not bind', () => {
  it('ignores every chord carrying Meta', () => {
    for (const context of CONTEXTS) {
      expect(resolveKey(context, ev({ key: 'Enter', code: 'Enter', metaKey: true }))).toBeNull();
      expect(
        resolveKey(context, ev({ key: ' ', code: 'Space', altKey: true, metaKey: true })),
      ).toBeNull();
    }
  });
});
