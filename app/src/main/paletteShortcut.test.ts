import { describe, expect, it, vi } from 'vitest';
import { IPC_OPEN_PALETTE, IPC_SHORTCUT_STATE, type ShortcutState } from '../shared/channels.js';
import {
  activateFromTray,
  installResidentShortcut,
  shortcutStateOf,
  startShortcutService,
  withShortcutRebind,
} from './paletteShortcut.js';
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

// R32 closed at the composition, not at the class. The publisher existed and was called from
// nowhere, so the drawer's chord row read `NOT SET` whatever the real binding was.
describe('startShortcutService', () => {
  it('sends the binding state on the shortcut channel before the drawer could ask', () => {
    const sent: { channel: string; payload: unknown }[] = [];
    const service = startShortcutService({
      host: host(),
      showWindow: vi.fn(),
      send: (channel, payload) => sent.push({ channel, payload }),
    });
    service.apply('Control+Shift+K');

    const states = sent
      .filter((message) => message.channel === IPC_SHORTCUT_STATE)
      .map((message) => message.payload as ShortcutState);
    expect(states.at(-1)).toEqual({ chord: 'Control+Shift+K', registered: true });
    // The real binding, not `null`: an unbound frame here is indistinguishable from silence.
    expect(states.at(-1)?.chord).not.toBeNull();
  });

  it('sends the failed registration rather than sending nothing', () => {
    const sent: { channel: string; payload: unknown }[] = [];
    const service = startShortcutService({
      host: host(false),
      showWindow: vi.fn(),
      send: (channel, payload) => sent.push({ channel, payload }),
    });
    service.apply('Control+Shift+K');
    expect(
      sent.filter((message) => message.channel === IPC_SHORTCUT_STATE).at(-1)?.payload,
    ).toEqual({ chord: 'Control+Shift+K', registered: false });
  });

  it('opens the palette on the palette channel, and shows the window first', () => {
    const sent: string[] = [];
    const order: string[] = [];
    const h = host();
    const service = startShortcutService({
      host: h,
      showWindow: () => order.push('show'),
      send: (channel) => {
        sent.push(channel);
        if (channel === IPC_OPEN_PALETTE) order.push('palette');
      },
    });
    service.apply('Control+Shift+K');
    h.fire();
    expect(order).toEqual(['show', 'palette']);
    expect(sent).toContain(IPC_OPEN_PALETTE);
  });
});

describe('withShortcutRebind', () => {
  it('re-applies the chord the drawer just wrote, so the row is not a dead switch', async () => {
    const applied: (string | null)[] = [];
    const request = withShortcutRebind(
      (name) =>
        Promise.resolve(
          name === 'settings.set' ? { residentShortcut: 'Alt+F12' } : { residentShortcut: null },
        ),
      (chord) => {
        applied.push(chord);
        return { kind: 'bound' as const, chord: chord ?? '' };
      },
    );

    await request('settings.set', { patch: {} });
    expect(applied).toEqual(['Alt+F12']);
  });

  it('leaves every other command untouched', async () => {
    const applied: (string | null)[] = [];
    const request = withShortcutRebind(
      () => Promise.resolve({ residentShortcut: 'Alt+F12' }),
      (chord) => {
        applied.push(chord);
        return { kind: 'unbound' as const };
      },
    );
    await request('settings.get', {});
    expect(applied).toEqual([]);
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
