/**
 * §48.5 item 1: with no updater, the app states which build it is. This half proves the line the
 * shell stamps carries the version the workspace declares, and that it reaches the renderer's
 * bridge unchanged. The drawer's half — that the line is drawn — is
 * `src/renderer/settings/versionLine.test.tsx`, because mounting the drawer needs the DOM project
 * and reading the packaged manifest needs this one.
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { formatArtifactStamp, readArtifactStamp } from '../src/main/update/artifact';
import type { CodothecaBridge } from '../src/shared/bridge';
import { buildStampArgument } from '../src/shared/windowArgs';

// `directories.app` is the repository root (`electron-builder.yml`), so the packaged asar's
// `package.json` — the one `app.getAppPath()` reads — is the root manifest.
const repoRoot = fileURLToPath(new URL('../../', import.meta.url));

const exposed = new Map<string, unknown>();

vi.mock('electron', () => ({
  contextBridge: {
    exposeInMainWorld: (key: string, value: unknown) => {
      exposed.set(key, value);
    },
  },
  ipcRenderer: { invoke: () => Promise.resolve(null), on: () => undefined },
}));

const REAL_ARGV = process.argv;

afterEach(() => {
  process.argv = REAL_ARGV;
});

async function loadPreload(argv: readonly string[]): Promise<CodothecaBridge> {
  process.argv = [...argv];
  exposed.clear();
  vi.resetModules();
  await import('../src/preload/index');
  return exposed.get('codotheca') as CodothecaBridge;
}

describe('the version line', () => {
  it('AC-P4-48-25 the stamped version equals the root package.json', async () => {
    const manifest: unknown = JSON.parse(readFileSync(`${repoRoot}package.json`, 'utf8'));
    const declared = (manifest as { version?: unknown }).version;
    expect(typeof declared).toBe('string');

    const stamp = readArtifactStamp({
      appPath: repoRoot,
      readTextFile: (p) => readFileSync(p, 'utf8'),
      artifact: { isPackaged: true, platform: 'win32', env: {} },
    });
    expect(stamp).toEqual({ version: declared, kind: 'nsis' });

    // The line crosses to the renderer the way the shell sends it: on the command line, read
    // back by the preload with no round trip.
    const line = formatArtifactStamp(stamp);
    const bridge = await loadPreload(['electron', buildStampArgument(line)]);
    expect(bridge.buildStamp).toBe(line);
    expect(bridge.buildStamp.startsWith(`${String(declared)} · `)).toBe(true);
  });

  it('carries no line at all when the shell passed none', async () => {
    const bridge = await loadPreload(['electron']);
    expect(bridge.buildStamp).toBe('');
  });
});
