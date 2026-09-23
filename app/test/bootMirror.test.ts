import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { type Mock, afterEach, describe, expect, it, vi } from 'vitest';
import { mirrorOnJoin, mirrorShelf, noteForcedOff, tierMirror } from '../src/main/bootMirror';
import { bootFilePath, readBootFile, writeBootFile } from '../src/main/bootStore';
import { bootMirrorStep } from '../src/main/joinSteps';
import { onStoredSettings } from '../src/main/storedSettings';
import { DEFAULT_BOOT_FILE, type BootFile } from '../src/shared/bootFile';
import type { Settings } from '../src/generated/protocol';

function deps(stored: BootFile): {
  deps: { dataDir: string; readBoot: () => BootFile; writeBoot: Mock };
  writeBoot: Mock;
} {
  const writeBoot = vi.fn();
  return { deps: { dataDir: '/data', readBoot: () => stored, writeBoot }, writeBoot };
}

const settings: Settings = {
  effectsTier: 'reduced',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
  healthChecks: [],
};

const stored: BootFile = {
  generation: 2,
  effectsTier: 'full',
  paintFailCount: 0,
  paintFailForcedAt: null,
  reducedMotionOverride: false,
  shelfProjection: null,
};

describe('the boot mirror', () => {
  it('rewrites the file when the core disagrees with it', () => {
    const { deps: d, writeBoot } = deps(stored);
    const next = mirrorOnJoin(d, settings);
    expect(next.effectsTier).toBe('reduced');
    expect(writeBoot).toHaveBeenCalledTimes(1);
  });

  it('writes nothing when the file already agrees', () => {
    const { deps: d, writeBoot } = deps({ ...stored, effectsTier: 'reduced' });
    mirrorOnJoin(d, settings);
    expect(writeBoot).not.toHaveBeenCalled();
  });

  // The override clamps the tier, so a file carrying the tier alone would paint a launch's first
  // frame — and every frame, if the core never answers — at motion the user turned down.
  it('mirrors the reduced-motion override beside the tier', () => {
    const { deps: d, writeBoot } = deps({ ...stored, effectsTier: 'reduced' });
    const next = mirrorOnJoin(d, { ...settings, reducedMotionOverride: true });
    expect(next.reducedMotionOverride).toBe(true);
    expect(next.effectsTier).toBe('reduced');
    expect(writeBoot).toHaveBeenCalledTimes(1);
    expect((writeBoot.mock.calls[0]?.[1] as BootFile).reducedMotionOverride).toBe(true);
  });

  it('mirrors the shelf projection without disturbing the tier', () => {
    const { deps: d, writeBoot } = deps(stored);
    mirrorShelf(d, { rows: [] });
    const written = writeBoot.mock.calls[0]?.[1] as BootFile;
    expect(written.shelfProjection).toEqual({ rows: [] });
    expect(written.effectsTier).toBe('full');
  });

  it('records which launch forced the tier off so settings can name it', () => {
    const { deps: d, writeBoot } = deps({ ...stored, paintFailCount: 2 });
    noteForcedOff(d, 1_700_000_000_000);
    const written = writeBoot.mock.calls[0]?.[1] as BootFile;
    expect(written.effectsTier).toBe('off');
    expect(written.paintFailForcedAt).toBe(1_700_000_000_000);
  });

  it('the database stays authoritative — the join never writes the file back to the core', () => {
    // §11.2a: `boot.json` is a mirror. If the mirror won, a tier forced off by a broken GPU
    // would overwrite the setting the user chose, and settings would show a value nothing set.
    const { deps: d } = deps({ ...stored, effectsTier: 'off' });
    expect(mirrorOnJoin(d, settings).effectsTier).toBe('reduced');
  });

  it('a mirrored shelf keeps the forcing record the drawer reads', () => {
    const { deps: d, writeBoot } = deps({ ...stored, paintFailForcedAt: 42 });
    mirrorShelf(d, { rows: [1] });
    expect((writeBoot.mock.calls[0]?.[1] as BootFile).paintFailForcedAt).toBe(42);
  });
});

/**
 * §11.2a's mirror existed and nothing called it: `boot.json` kept `auto` on every install, so a
 * tier the user stored never reached the first frame of any later launch — the one moment the
 * setting exists for, since the GPU is what may be broken.
 */
describe('the boot mirror runs at join and on every stored write', () => {
  const dirs: string[] = [];
  afterEach(() => {
    for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  });

  function dataDir(): string {
    const dir = mkdtempSync(join(tmpdir(), 'codotheca-boot-'));
    dirs.push(dir);
    writeBootFile(dir, DEFAULT_BOOT_FILE);
    return dir;
  }

  const onDisk = (dir: string): { effects_tier?: unknown; reduced_motion_override?: unknown } =>
    JSON.parse(readFileSync(bootFilePath(dir), 'utf8')) as {
      effects_tier?: unknown;
      reduced_motion_override?: unknown;
    };

  it('the join step writes the stored tier and override into boot.json on disk', async () => {
    const dir = dataDir();
    const request = vi.fn(() =>
      Promise.resolve({ ...settings, effectsTier: 'off', reducedMotionOverride: true }),
    );
    const step = bootMirrorStep(
      request,
      tierMirror({ dataDir: dir, readBoot: readBootFile, writeBoot: writeBootFile }, vi.fn()),
    );
    expect(onDisk(dir).effects_tier).toBe('auto');
    await step.run();
    expect(request).toHaveBeenCalledWith('settings.get', {});
    expect(onDisk(dir)).toMatchObject({ effects_tier: 'off', reduced_motion_override: true });
  });

  it('a stored write is mirrored as the core answered it, and its answer passes through', async () => {
    const dir = dataDir();
    const request = vi.fn((name: string) =>
      Promise.resolve(name === 'settings.set' ? { ...settings, effectsTier: 'reduced' } : null),
    );
    const mirrored = onStoredSettings(
      request,
      tierMirror({ dataDir: dir, readBoot: readBootFile, writeBoot: writeBootFile }, vi.fn()),
    );
    await mirrored('settings.get', {});
    expect(onDisk(dir).effects_tier).toBe('auto');
    const answer = await mirrored('settings.set', { patch: {} });
    expect((answer as Settings).effectsTier).toBe('reduced');
    expect(onDisk(dir).effects_tier).toBe('reduced');
  });

  // The core has stored the write by the time the file is touched. A mirror that threw would
  // reject a write that succeeded, and the drawer would go on showing the old tier.
  it('a mirror that cannot write reports it and never fails the write it follows', async () => {
    const dir = dataDir();
    writeFileSync(bootFilePath(dir), 'not json', 'utf8');
    const onError = vi.fn();
    const mirror = tierMirror(
      {
        dataDir: dir,
        readBoot: readBootFile,
        writeBoot: () => {
          throw new Error('disk full');
        },
      },
      onError,
    );
    const mirrored = onStoredSettings(() => Promise.resolve(settings), mirror);
    await expect(mirrored('settings.set', { patch: {} })).resolves.toEqual(settings);
    expect(onError).toHaveBeenCalledTimes(1);
  });

  // Most writes are a check switch, the roast or the shortcut. None moves the file, and a
  // synchronous read of it on each one blocks the main process for nothing.
  it('reads boot.json only when the tier or the override moved since it last looked', () => {
    let file: BootFile = { ...stored, effectsTier: 'auto' };
    const readBoot = vi.fn(() => file);
    const writeBoot = vi.fn((_dir: string, next: BootFile) => {
      file = next;
    });
    const mirror = tierMirror({ dataDir: '/data', readBoot, writeBoot }, vi.fn());
    const counts = (): number[] => [readBoot.mock.calls.length, writeBoot.mock.calls.length];

    mirror(settings);
    expect(counts()).toEqual([1, 1]);
    mirror({ ...settings, roastEnabled: false });
    mirror({ ...settings, residentShortcut: 'Alt+F12' });
    expect(counts()).toEqual([1, 1]);
    mirror({ ...settings, reducedMotionOverride: true });
    expect(counts()).toEqual([2, 2]);
    expect(file.reducedMotionOverride).toBe(true);
    mirror({ ...settings, effectsTier: 'off', reducedMotionOverride: true });
    expect(counts()).toEqual([3, 3]);
    expect(file.effectsTier).toBe('off');
  });

  // A write that failed left the file where it was, so the next answer has to try again.
  it('a failed write is retried on the next answer rather than remembered as done', () => {
    let fail = true;
    const writeBoot = vi.fn(() => {
      if (fail) throw new Error('disk full');
    });
    const onError = vi.fn();
    const mirror = tierMirror({ dataDir: '/data', readBoot: () => stored, writeBoot }, onError);
    mirror(settings);
    fail = false;
    mirror(settings);
    expect(writeBoot).toHaveBeenCalledTimes(2);
    expect(onError).toHaveBeenCalledTimes(1);
  });
});

/**
 * The production caller, read from the shell's entry point. Importing `src/main/index.ts` pulls in
 * Electron, which vitest's node project cannot load, so the wiring is checked on the syntax tree:
 * a comment or a string naming either function matches nothing here.
 */
describe('the shell wires the mirror', () => {
  const file = fileURLToPath(new URL('../src/main/index.ts', import.meta.url));
  const source = ts.createSourceFile(
    file,
    readFileSync(file, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
  );

  function callsTo(root: ts.Node, name: string): ts.CallExpression[] {
    const found: ts.CallExpression[] = [];
    const visit = (node: ts.Node): void => {
      if (
        ts.isCallExpression(node) &&
        ts.isIdentifier(node.expression) &&
        node.expression.text === name
      ) {
        found.push(node);
      }
      ts.forEachChild(node, visit);
    };
    visit(root);
    return found;
  }

  function property(call: ts.CallExpression, name: string): ts.Node | undefined {
    const [argument] = call.arguments;
    if (argument === undefined || !ts.isObjectLiteralExpression(argument)) return undefined;
    return argument.properties.find(
      (p) => p.name !== undefined && ts.isIdentifier(p.name) && p.name.text === name,
    );
  }

  it('runs the mirror as a join step', () => {
    const [startup] = callsTo(source, 'runStartup');
    expect(startup, 'index.ts calls runStartup').toBeDefined();
    const steps = startup === undefined ? undefined : property(startup, 'joinSteps');
    expect(steps, 'runStartup is given joinSteps').toBeDefined();
    expect(steps === undefined ? [] : callsTo(steps, 'bootMirrorStep')).toHaveLength(1);
  });

  it('hands the renderer a request that mirrors every stored write', () => {
    const [bridge] = callsTo(source, 'registerBridge');
    const given = bridge === undefined ? undefined : property(bridge, 'request');
    expect(given, 'registerBridge is given a request').toBeDefined();
    // `request` is passed by name; its one declaration is what the renderer's writes go through.
    const declarations: ts.VariableDeclaration[] = [];
    const visit = (node: ts.Node): void => {
      if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
        if (node.name.text === 'request') declarations.push(node);
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
    expect(given !== undefined && ts.isShorthandPropertyAssignment(given)).toBe(true);
    expect(declarations).toHaveLength(1);
    const initializer = declarations[0]?.initializer;
    const wrapped =
      initializer !== undefined &&
      ts.isCallExpression(initializer) &&
      ts.isIdentifier(initializer.expression) &&
      initializer.expression.text === 'onStoredSettings'
        ? initializer
        : undefined;
    expect(wrapped, 'request is built by onStoredSettings').toBeDefined();
    const listeners = wrapped?.arguments.slice(1) ?? [];
    expect(listeners.some((arg) => ts.isIdentifier(arg) && arg.text === 'mirrorTier')).toBe(true);
  });

  // The override reaches the first frame on the window's argv, beside the tier (§11.2a).
  it('puts the boot values on the window through bootArguments', () => {
    const found: ts.Node[] = [];
    const visit = (node: ts.Node): void => {
      if (
        ts.isPropertyAssignment(node) &&
        ts.isIdentifier(node.name) &&
        node.name.text === 'additionalArguments'
      ) {
        found.push(node.initializer);
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
    expect(found).toHaveLength(1);
    const [list] = found;
    expect(list === undefined ? [] : callsTo(list, 'bootArguments')).toHaveLength(1);
  });
});
