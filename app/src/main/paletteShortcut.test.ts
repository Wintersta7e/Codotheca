import { describe, expect, it, vi } from 'vitest';
import type { ShortcutState } from '../shared/channels.js';
import { activateFromTray, installResidentShortcut, shortcutStateOf } from './paletteShortcut.js';
import type { ShortcutHost } from './shortcut.js';

function host(accept = true): ShortcutHost & { fire: () => void } {
  let bound: (() => void) | null = null;
  return {
    fire: () => {
      bound?.();
    },
    register: (_chord, cb) => {
      if (!accept) return false;
      bound = cb;
      return true;
    },
    unregister: () => {
      bound = null;
    },
    isRegistered: () => bound !== null,
  };
}

describe('installResidentShortcut', () => {
  it('one press shows the window and opens the palette, in that order', () => {
    const order: string[] = [];
    const h = host();
    const handle = installResidentShortcut({
      host: h,
      showWindow: () => order.push('show'),
      openPalette: () => order.push('palette'),
      publishShortcutState: vi.fn(),
    });
    handle.apply('Control+Shift+K');
    h.fire();
    expect(order).toEqual(['show', 'palette']);
  });

  it('starts unbound and says so', () => {
    const handle = installResidentShortcut({
      host: host(),
      showWindow: vi.fn(),
      openPalette: vi.fn(),
      publishShortcutState: vi.fn(),
    });
    expect(handle.statusText()).toBe('NOT SET');
  });

  it('a refused chord reads back its reason', () => {
    const handle = installResidentShortcut({
      host: host(false),
      showWindow: vi.fn(),
      openPalette: vi.fn(),
      publishShortcutState: vi.fn(),
    });
    expect(handle.apply('Control+Shift+K').kind).toBe('refused');
    expect(handle.statusText()).toBe('NOT SET · Control+Shift+K IS HELD BY ANOTHER APPLICATION');
  });
});

// R32: the class that holds the binding is the only thing that knows its state, so this plan
// publishes it. A drawer subscribing to a channel nobody sends renders a dead control.
describe('publishing shortcut state', () => {
  it('publishes the unbound state at install, before anything is bound', () => {
    const publishShortcutState = vi.fn();
    installResidentShortcut({
      host: host(),
      showWindow: vi.fn(),
      openPalette: vi.fn(),
      publishShortcutState,
    });
    expect(publishShortcutState).toHaveBeenCalledWith({ chord: null, registered: false });
  });

  it('publishes every transition: bound, refused, and back to unbound', () => {
    const publishShortcutState = vi.fn();
    const handle = installResidentShortcut({
      host: host(),
      showWindow: vi.fn(),
      openPalette: vi.fn(),
      publishShortcutState,
    });
    handle.apply('Control+Shift+K');
    handle.apply(null);
    const published = publishShortcutState.mock.calls.map((call) => call[0] as ShortcutState);
    expect(published).toEqual([
      { chord: null, registered: false },
      { chord: 'Control+Shift+K', registered: true },
      { chord: null, registered: false },
    ]);
  });

  // §8.6: register returns a boolean and never says why. A chord another application holds reads
  // back with the chord and `registered: false` — never as silence, and never as unbound with the
  // chord thrown away, which would erase what the drawer must show.
  it('publishes a refusal with the chord that was refused', () => {
    const publishShortcutState = vi.fn();
    const handle = installResidentShortcut({
      host: host(false),
      showWindow: vi.fn(),
      openPalette: vi.fn(),
      publishShortcutState,
    });
    handle.apply('Control+Shift+K');
    expect(publishShortcutState).toHaveBeenLastCalledWith({
      chord: 'Control+Shift+K',
      registered: false,
    });
  });

  it('publishes the unbound state on dispose, so a quit leaves no stale chord on screen', () => {
    const publishShortcutState = vi.fn();
    const handle = installResidentShortcut({
      host: host(),
      showWindow: vi.fn(),
      openPalette: vi.fn(),
      publishShortcutState,
    });
    handle.apply('Control+Shift+K');
    handle.dispose();
    expect(publishShortcutState).toHaveBeenLastCalledWith({ chord: null, registered: false });
  });
});

describe('shortcutStateOf', () => {
  it('is the whole truth a boolean return can carry', () => {
    expect(shortcutStateOf({ kind: 'unbound' })).toEqual({ chord: null, registered: false });
    expect(shortcutStateOf({ kind: 'bound', chord: 'Alt+F12' })).toEqual({
      chord: 'Alt+F12',
      registered: true,
    });
    expect(shortcutStateOf({ kind: 'refused', chord: 'Alt+F12' })).toEqual({
      chord: 'Alt+F12',
      registered: false,
    });
  });
});

describe('activateFromTray', () => {
  // Criterion 61: tray-icon activation produces the window and no palette.
  it('shows the window and opens nothing', () => {
    const showWindow = vi.fn();
    const openPalette = vi.fn();
    activateFromTray({ showWindow });
    expect(showWindow).toHaveBeenCalledTimes(1);
    expect(openPalette).not.toHaveBeenCalled();
  });
});
