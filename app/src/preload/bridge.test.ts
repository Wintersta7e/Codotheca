import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { CodothecaBridge } from '../shared/bridge';
import { logPathArgument } from '../shared/windowArgs';

const exposed = new Map<string, unknown>();

vi.mock('electron', () => ({
  contextBridge: {
    exposeInMainWorld: (key: string, value: unknown) => {
      exposed.set(key, value);
    },
  },
  ipcRenderer: {
    invoke: () => Promise.resolve(null),
    on: () => undefined,
  },
}));

const REAL_ARGV = process.argv;

async function loadPreload(argv: readonly string[]): Promise<CodothecaBridge> {
  process.argv = [...argv];
  exposed.clear();
  vi.resetModules();
  await import('./index');
  const bridge = exposed.get('codotheca');
  expect(bridge).toBeDefined();
  return bridge as CodothecaBridge;
}

beforeEach(() => {
  exposed.clear();
});

afterEach(() => {
  process.argv = REAL_ARGV;
});

describe('the preload bridge', () => {
  it('exposes a non-empty log path, so the failure windows have one to name', () => {
    const path = '/home/u/.config/Codotheca/logs/codotheca.log';
    return loadPreload(['electron', logPathArgument(path)]).then((bridge) => {
      // §11.2a names the log on every failure window. It is a *display* string: the renderer
      // cannot open it and never originates one (§2.4).
      expect(bridge.logPath).toBe(path);
      expect(bridge.logPath.length).toBeGreaterThan(0);
    });
  });

  it('carries the tier beside it, from the same argv, with no round trip', () =>
    loadPreload(['electron', logPathArgument('/tmp/x.log'), '--effects-tier=reduced']).then(
      (bridge) => {
        expect(bridge.effectsTier).toBe('reduced');
        expect(bridge.logPath).toBe('/tmp/x.log');
      },
    ));
});
