import { describe, expect, it, vi } from 'vitest';
import { XDG_AUTOSTART_FILE, autostartFor, desktopEntry } from '../src/main/autostart';

interface FakeFs {
  readonly files: Map<string, string>;
  readonly write: (p: string, t: string) => void;
  readonly remove: (p: string) => void;
  readonly exists: (p: string) => boolean;
  readonly mkdirp: (p: string) => void;
}

function fakeFs(): FakeFs {
  const files = new Map<string, string>();
  return {
    files,
    write: (p: string, t: string): void => void files.set(p, t),
    remove: (p: string): void => void files.delete(p),
    exists: (p: string): boolean => files.has(p),
    mkdirp: vi.fn(),
  };
}

describe('autostart', () => {
  it('uses the OS login item on Windows', () => {
    let on = false;
    const port = autostartFor({
      platform: 'win32',
      loginItem: {
        get: () => on,
        set: (v) => {
          on = v;
        },
      },
      configHome: '/config',
      execPath: '/app/codotheca',
      fs: fakeFs(),
    });
    expect(port.get()).toBe(false);
    port.set(true);
    expect(on).toBe(true);
    expect(port.get()).toBe(true);
  });

  it('writes and removes a real XDG entry on Linux, where the login item is a no-op', () => {
    const fs = fakeFs();
    const loginItem = { get: () => false, set: vi.fn() };
    const port = autostartFor({
      platform: 'linux',
      loginItem,
      configHome: '/config',
      execPath: '/app/codotheca',
      fs,
    });
    port.set(true);
    expect(fs.files.get(`/config/autostart/${XDG_AUTOSTART_FILE}`)).toContain('Type=Application');
    expect(port.get()).toBe(true);
    // A switch that silently does nothing is the dead switch §11.3a forbids.
    expect(loginItem.set).not.toHaveBeenCalled();
    port.set(false);
    expect(port.get()).toBe(false);
    expect(fs.files.size).toBe(0);
  });

  it('creates the autostart directory before writing into it', () => {
    const fs = fakeFs();
    const port = autostartFor({
      platform: 'linux',
      loginItem: { get: () => false, set: vi.fn() },
      configHome: '/config',
      execPath: '/app/codotheca',
      fs,
    });
    port.set(true);
    expect(fs.mkdirp).toHaveBeenCalledWith('/config/autostart');
  });

  it('turning it off when it was never on is not an error', () => {
    const fs = fakeFs();
    const port = autostartFor({
      platform: 'linux',
      loginItem: { get: () => false, set: vi.fn() },
      configHome: '/config',
      execPath: '/app/codotheca',
      fs,
    });
    expect(() => {
      port.set(false);
    }).not.toThrow();
    expect(port.get()).toBe(false);
  });

  it('the desktop entry starts hidden and carries no telemetry or account hint', () => {
    const text = desktopEntry('/app/codotheca');
    expect(text).toContain('Exec=/app/codotheca --hidden');
    expect(text).toContain('X-GNOME-Autostart-enabled=true');
    expect(text.split('\n')[0]).toBe('[Desktop Entry]');
    // §0: no account, no telemetry — and the entry is one of the few files that leaves the app.
    expect(text).not.toMatch(/telemetry|account|analytics|http/i);
  });
});
