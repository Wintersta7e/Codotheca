// The four things the renderer cannot do for itself.
//
// §2.4: the renderer may never originate a filesystem path or an executable. `roots.add` and
// `targets.upsert` are privileged, so `isRendererCallable` refuses them — the drawer's
// `ADD A FOLDER` and its `CHANGE` control reach them only through a shell-owned native dialog,
// which is what these channels are.
import { join } from 'node:path';
import type { BootFile } from '../shared/bootFile';
import {
  INDEX_DB_FILE,
  IPC_CLEAR_PAINT_FAILURE,
  IPC_INDEX_LOCATION,
  IPC_PICK_EXECUTABLE,
  IPC_REVEAL,
  type IndexLocation,
  type RevealTarget,
} from '../shared/channels';

export interface ShellServiceDeps {
  handle(channel: string, fn: (event: unknown, args: unknown) => unknown): void;
  // R11: no `openDirectory` — plan 16's rootPicker owns the directory dialog and the only
  // `ipcMain.handle` for `IPC_PICK_ROOT`. A second registration throws at startup.
  /** Native executable dialog. `null` when the user cancelled. */
  openExecutable(): Promise<Buffer | null>;
  revealItem(path: string): void;
  dataDir: string;
  /** The rolling log's own path. §11.1's `OPEN THE LOG` names a target, never a path. */
  logPath: string;
  statSync(path: string): { size: number };
  request(name: string, args: unknown): Promise<unknown>;
  readBoot(dataDir: string): BootFile;
  writeBoot(dataDir: string, file: BootFile): void;
}

/**
 * Read from a `stat`, never from an open connection: §1.10 gives the core the only database
 * handle, and the shell reporting a size is not a reason to break that.
 */
export function indexLocation(
  dataDir: string,
  stat: (p: string) => { size: number },
): IndexLocation {
  const path = join(dataDir, INDEX_DB_FILE);
  return { pathDisplay: path, sizeBytes: stat(path).size };
}

export function registerShellServices(deps: ShellServiceDeps): void {
  // R11: `IPC_PICK_ROOT` is deliberately absent. Plan 16's `app/src/main/rootPicker.ts`
  // registers it and answers `PickRootReply`; registering it twice throws at startup.

  deps.handle(IPC_PICK_EXECUTABLE, async (_event, args) => {
    const bytes = await deps.openExecutable();
    if (bytes === null) return { ok: true, value: null };
    const scope = (args ?? {}) as Record<string, unknown>;
    const value = await deps.request('targets.upsert', {
      targetId: scope['targetId'] ?? null,
      kind: scope['kind'] ?? 'editor',
      name: scope['name'] ?? '',
      execBytes: { b64: bytes.toString('base64') },
      argv: scope['argv'] ?? [],
      projectId: scope['projectId'] ?? null,
      locationId: scope['locationId'] ?? null,
      language: scope['language'] ?? null,
    });
    return { ok: true, value };
  });

  deps.handle(IPC_REVEAL, (_event, args) => {
    // §2.4: the renderer never originates a path. It names one of two targets, and anything
    // that is not one of them reveals nothing at all.
    const target = (args as { target?: unknown } | null)?.target as RevealTarget | undefined;
    if (target === 'index') deps.revealItem(join(deps.dataDir, INDEX_DB_FILE));
    else if (target === 'bundle') deps.revealItem(deps.dataDir);
    else if (target === 'log') deps.revealItem(deps.logPath);
    return { ok: true, value: null };
  });

  deps.handle(IPC_INDEX_LOCATION, () => ({
    ok: true,
    value: indexLocation(deps.dataDir, (p) => deps.statSync(p)),
  }));

  deps.handle(IPC_CLEAR_PAINT_FAILURE, () => {
    // Reachable from settings: a user who cannot see the window cannot change a setting
    // inside it. The forcing timestamp clears with the count, or the drawer goes on naming a
    // launch that no longer forces anything.
    const stored = deps.readBoot(deps.dataDir);
    deps.writeBoot(deps.dataDir, { ...stored, paintFailCount: 0, paintFailForcedAt: null });
    return { ok: true, value: null };
  });
}
