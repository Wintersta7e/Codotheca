import { describe, expect, it, vi } from 'vitest';
import {
  ResidentShortcut,
  SYSTEM_MENU_CONSEQUENCE,
  chordProposalWarning,
  disableDefaultApplicationMenu,
  isSystemWindowMenuChord,
  recordChord,
  residentShortcutStatusText,
} from './shortcut.js';
import type { ChordDescriptor, ShortcutHost } from './shortcut.js';

const chord = (over: Partial<ChordDescriptor>): ChordDescriptor => ({
  key: 'K',
  code: 'KeyK',
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...over,
});

function fakeHost(register: (chord: string) => boolean): ShortcutHost & {
  readonly registered: Set<string>;
} {
  const registered = new Set<string>();
  return {
    registered,
    register: (c) => {
      if (!register(c)) return false;
      registered.add(c);
      return true;
    },
    unregister: (c) => {
      registered.delete(c);
    },
    isRegistered: (c) => registered.has(c),
  };
}

/** The system-menu chord, assembled the way the product assembles it — never spelled here. */
const systemMenuChord = (): string => {
  const recorded = recordChord(chord({ altKey: true, code: 'Space', key: ' ' }));
  if (recorded === null) throw new Error('the recorder refused a chord it must accept');
  return recorded;
};

describe('recordChord', () => {
  it('builds an Electron accelerator from modifiers plus one key', () => {
    expect(recordChord(chord({ ctrlKey: true, shiftKey: true }))).toBe('Control+Shift+K');
    expect(recordChord(chord({ altKey: true, code: 'F12', key: 'F12' }))).toBe('Alt+F12');
    expect(recordChord(chord({ ctrlKey: true, code: 'Digit4', key: '4' }))).toBe('Control+4');
  });

  it('refuses a bare key and a bare modifier, which are not chords', () => {
    expect(recordChord(chord({}))).toBeNull();
    expect(recordChord(chord({ code: 'AltLeft', key: 'Alt', altKey: true }))).toBeNull();
    expect(recordChord(chord({ code: 'ShiftLeft', key: 'Shift', shiftKey: true }))).toBeNull();
  });
});

describe('isSystemWindowMenuChord', () => {
  // §11.7: the chord is never a literal in the shell bundle, so it is derived, not compared.
  it('recognises the system window menu chord from its parts', () => {
    expect(isSystemWindowMenuChord(systemMenuChord())).toBe(true);
  });

  it('does not recognise anything else', () => {
    expect(isSystemWindowMenuChord('Control+Space')).toBe(false);
    expect(isSystemWindowMenuChord('Alt+Shift+Space')).toBe(false);
    expect(isSystemWindowMenuChord('Alt+K')).toBe(false);
  });

  it('states the consequence rather than refusing the chord outright', () => {
    expect(chordProposalWarning(systemMenuChord())).toBe(SYSTEM_MENU_CONSEQUENCE);
    expect(SYSTEM_MENU_CONSEQUENCE).toBe(
      'EVERY OTHER WINDOW LOSES ITS ALT+SPACE MENU WHILE CODOTHECA RUNS',
    );
    expect(chordProposalWarning('Control+Shift+K')).toBeNull();
  });
});

describe('ResidentShortcut', () => {
  it('registers nothing until a chord is bound', () => {
    const host = fakeHost(() => true);
    const shortcut = new ResidentShortcut(host, vi.fn());
    expect(shortcut.outcome).toEqual({ kind: 'unbound' });
    expect(host.registered.size).toBe(0);
    expect(residentShortcutStatusText(shortcut.outcome)).toBe('NOT SET');
  });

  it('binds, and the press fires the callback', () => {
    let bound: (() => void) | null = null;
    const press = vi.fn();
    const host: ShortcutHost = {
      register: (_c, cb) => {
        bound = cb;
        return true;
      },
      unregister: vi.fn(),
      isRegistered: () => bound !== null,
    };
    const shortcut = new ResidentShortcut(host, press);
    expect(shortcut.bind('Control+Shift+K')).toEqual({ kind: 'bound', chord: 'Control+Shift+K' });
    const fire = bound as (() => void) | null;
    if (fire === null) throw new Error('the host was never handed a callback');
    fire();
    expect(press).toHaveBeenCalledTimes(1);
  });

  // §8.6: register returns a boolean and does not throw. A failure is never silent.
  it('reports a refusal with its reason and registers nothing', () => {
    const host = fakeHost(() => false);
    const shortcut = new ResidentShortcut(host, vi.fn());
    expect(shortcut.bind('Control+Shift+K')).toEqual({
      kind: 'refused',
      chord: 'Control+Shift+K',
    });
    expect(host.registered.size).toBe(0);
    expect(residentShortcutStatusText(shortcut.outcome)).toBe(
      'NOT SET · Control+Shift+K IS HELD BY ANOTHER APPLICATION',
    );
  });

  it('releases the previous chord before taking a new one', () => {
    const host = fakeHost(() => true);
    const shortcut = new ResidentShortcut(host, vi.fn());
    shortcut.bind('Control+Shift+K');
    shortcut.bind('Control+Shift+J');
    expect([...host.registered]).toEqual(['Control+Shift+J']);
    shortcut.bind(null);
    expect(host.registered.size).toBe(0);
    expect(shortcut.outcome).toEqual({ kind: 'unbound' });
  });

  it('binds the system-menu chord when the user deliberately asks for it', () => {
    const host = fakeHost(() => true);
    const shortcut = new ResidentShortcut(host, vi.fn());
    const recorded = systemMenuChord();
    expect(shortcut.bind(recorded)).toEqual({ kind: 'bound', chord: recorded });
    expect(host.registered.has(recorded)).toBe(true);
  });

  // A refusal that kept the old registration would leave the shortcut doing one thing and the
  // settings row saying another.
  it('a refused rebind releases the chord it was holding', () => {
    let accept = true;
    const host = fakeHost(() => accept);
    const shortcut = new ResidentShortcut(host, vi.fn());
    shortcut.bind('Control+Shift+K');
    accept = false;
    expect(shortcut.bind('Control+Shift+J').kind).toBe('refused');
    expect(host.registered.size).toBe(0);
  });
});

describe('disableDefaultApplicationMenu', () => {
  // §8.8: the shell drops the default accelerator so Ctrl+S cannot reach a Save-Page dialog.
  it('clears the application menu', () => {
    const setApplicationMenu = vi.fn();
    disableDefaultApplicationMenu(setApplicationMenu);
    expect(setApplicationMenu).toHaveBeenCalledWith(null);
  });
});
