import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { type Mock, describe, expect, it, vi } from 'vitest';
import { indexLocation, registerShellServices } from '../src/main/shellServices';
// R11: no `IPC_PICK_ROOT` here — plan 16's rootPicker owns that channel and its handler.
import {
  INDEX_DB_FILE,
  IPC_CLEAR_PAINT_FAILURE,
  IPC_INDEX_LOCATION,
  IPC_PICK_EXECUTABLE,
  IPC_REVEAL,
} from '../src/shared/channels';
import type { BootFile } from '../src/shared/bootFile';

type Handler = (event: unknown, args: unknown) => unknown;

function boot(over: Partial<BootFile> = {}): BootFile {
  return {
    generation: 2,
    effectsTier: 'auto',
    paintFailCount: 0,
    paintFailForcedAt: null,
    shelfProjection: null,
    ...over,
  };
}

interface Registered {
  readonly handlers: Map<string, Handler>;
  readonly writeBoot: Mock;
  readonly revealItem: Mock;
  readonly request: Mock;
}

function register(over: Partial<Parameters<typeof registerShellServices>[0]> = {}): Registered {
  const handlers = new Map<string, Handler>();
  const writeBoot = vi.fn();
  const revealItem = vi.fn();
  const request = vi.fn(() => Promise.resolve(null));
  registerShellServices({
    handle: (channel, fn) => handlers.set(channel, fn),
    openExecutable: () => Promise.resolve(null),
    revealItem,
    dataDir: '/data',
    statSync: () => ({ size: 1 }),
    request,
    readBoot: () => boot(),
    writeBoot,
    ...over,
  });
  return { handlers, writeBoot, revealItem, request };
}

describe('the shell services', () => {
  it('reports the index location from a stat, never from an open connection', () => {
    const stat = vi.fn(() => ({ size: 4_194_304 }));
    const loc = indexLocation('/data', stat);
    expect(loc.sizeBytes).toBe(4_194_304);
    expect(loc.pathDisplay.endsWith(INDEX_DB_FILE)).toBe(true);
    expect(stat).toHaveBeenCalledTimes(1);
  });

  // R11: the `a cancelled folder dialog issues no privileged command` test belongs with the
  // handler, in plan 16's `app/test/rootPicker.test.ts`. Asserting a `{ok, value: null}` reply
  // here would test a shape plan 16's handler does not return.
  it('registers no handler for the root-picker channel', () => {
    const { handlers } = register();
    // Electron throws at a second ipcMain.handle for one channel — a startup crash, not a
    // duplicated style.
    expect([...handlers.keys()]).not.toContain('codotheca:pick-root');
    expect([...handlers.keys()].sort()).toEqual(
      [IPC_CLEAR_PAINT_FAILURE, IPC_INDEX_LOCATION, IPC_PICK_EXECUTABLE, IPC_REVEAL].sort(),
    );
  });

  it('reveal refuses a path the renderer supplied', async () => {
    const { handlers, revealItem } = register();
    await handlers.get(IPC_REVEAL)?.(null, { target: 'index' });
    await handlers.get(IPC_REVEAL)?.(null, { target: '/etc/shadow' });
    await handlers.get(IPC_REVEAL)?.(null, { target: '../../secrets' });
    await handlers.get(IPC_REVEAL)?.(null, null);
    // §2.4: the renderer names one of two targets. Anything else is not a path it may hand us.
    expect(revealItem).toHaveBeenCalledTimes(1);
    expect(revealItem).toHaveBeenCalledWith(join('/data', INDEX_DB_FILE));
  });

  it('a cancelled executable dialog issues no privileged command', async () => {
    const { handlers, request } = register({ openExecutable: () => Promise.resolve(null) });
    const reply = await handlers.get(IPC_PICK_EXECUTABLE)?.(null, { kind: 'editor' });
    expect(request).not.toHaveBeenCalled();
    expect(reply).toEqual({ ok: true, value: null });
  });

  it('a chosen executable reaches targets.upsert as bytes, never as renderer text', async () => {
    const { handlers, request } = register({
      openExecutable: () => Promise.resolve(Buffer.from('/opt/editor/bin')),
    });
    await handlers.get(IPC_PICK_EXECUTABLE)?.(null, { kind: 'editor', name: 'Editor' });
    expect(request).toHaveBeenCalledTimes(1);
    const call = request.mock.calls[0] as unknown as [string, Record<string, unknown>];
    expect(call[0]).toBe('targets.upsert');
    expect(call[1]['execBytes']).toEqual({
      b64: Buffer.from('/opt/editor/bin').toString('base64'),
    });
    expect(call[1]['kind']).toBe('editor');
  });

  it('clearing the paint failure also clears the launch that forced the tier off', () => {
    const { handlers, writeBoot } = register({
      readBoot: () => boot({ paintFailCount: 2, paintFailForcedAt: 5, effectsTier: 'off' }),
    });
    handlers.get(IPC_CLEAR_PAINT_FAILURE)?.(null, null);
    const written = writeBoot.mock.calls[0]?.[1] as BootFile;
    expect(written.paintFailCount).toBe(0);
    // Leaving the timestamp would keep settings naming a launch that no longer forced anything.
    expect(written.paintFailForcedAt).toBeNull();
  });

  it('answers the index location over its channel', () => {
    const { handlers } = register({ statSync: () => ({ size: 99 }) });
    const reply = handlers.get(IPC_INDEX_LOCATION)?.(null, null);
    expect(reply).toEqual({
      ok: true,
      value: { pathDisplay: join('/data', INDEX_DB_FILE), sizeBytes: 99 },
    });
  });
});

// R24: `INDEX_DB_FILE` is a cross-language mirror, and the only thing keeping it honest is a
// test that reads the other side. If the core renames the file, `REVEAL` opens a folder and
// highlights nothing, and the settings row reports a size for a file that is not there.
describe('the index file name', () => {
  it('equals the name the core actually opens', () => {
    const rust = readFileSync(
      fileURLToPath(new URL('../../core/src/index/mod.rs', import.meta.url)),
      'utf8',
    );
    const m = /pub fn db_path\(data_dir: &Path\) -> PathBuf \{\s*data_dir\.join\("([^"]+)"\)/.exec(
      rust,
    );
    expect(m, 'the core no longer declares db_path in the expected form').not.toBeNull();
    expect(INDEX_DB_FILE).toBe(m?.[1]);
  });
});
