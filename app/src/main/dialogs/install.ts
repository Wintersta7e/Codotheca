/**
 * The shell half of §24.3's clone, and of §24.3c's cancel.
 *
 * **There is no dialog here.** `IPC_RELOCATE` exists because a path must be originated by the
 * shell; these two channels originate nothing — the renderer supplies a `ProjectId` and a
 * `RootId`, and §24.3a composes the destination in the core from a root the user already added.
 * A folder that is not yet a root is reached through `IPC_PICK_ROOT`, which stays the only
 * path-origination channel the product has.
 *
 * What the channels are *for* is the reachability rule: `install.start` and `install.cancel`
 * mutate the filesystem, so they are `privileged` under §24.8's extended meaning and
 * `isRendererCallable` refuses both on `IPC_REQUEST`. `install.preview` is unprivileged and
 * rides `IPC_REQUEST` normally — it spawns no process and writes nothing.
 */
import {
  type BridgeError,
  IPC_INSTALL_CANCEL,
  IPC_INSTALL_START,
  type InstallCancelReply,
  type InstallStartReply,
} from '../../shared/channels';
import type { BridgeRequest } from '../core/bridge';

export interface InstallDeps {
  handle: (
    channel: string,
    fn: (payload: unknown) => Promise<InstallStartReply | InstallCancelReply>,
  ) => void;
  request: BridgeRequest;
}

/** The same validation `relocate.ts` applies, for the same reason: nothing else is passed on. */
function asId(payload: unknown, key: string): number | null {
  if (typeof payload !== 'object' || payload === null) return null;
  const id = (payload as Record<string, unknown>)[key];
  return typeof id === 'number' && Number.isSafeInteger(id) && id > 0 ? id : null;
}

/**
 * §2.2: `outcome` is `'unknown'` or absent. A call that definitely did not take effect carries
 * `null` — treating that as a word would tell the renderer a clone may have begun when the core
 * has just said it did not.
 */
function asBridgeError(err: unknown): BridgeError {
  const e = typeof err === 'object' && err !== null ? (err as Record<string, unknown>) : null;
  const code = e !== null && typeof e['code'] === 'string' ? e['code'] : 'INTERNAL';
  const message =
    e !== null && typeof e['message'] === 'string' ? e['message'] : 'could not complete';
  return {
    code: code as BridgeError['code'],
    message,
    outcome: e !== null && e['outcome'] === 'unknown' ? 'unknown' : null,
    retryable: e !== null && e['retryable'] === true,
  };
}

function badPayload(what: string): BridgeError {
  return { code: 'PROTOCOL', message: `bad ${what}`, outcome: null, retryable: false };
}

export function registerInstall(deps: InstallDeps): void {
  deps.handle(IPC_INSTALL_START, async (payload: unknown): Promise<InstallStartReply> => {
    const projectId = asId(payload, 'projectId');
    const rootId = asId(payload, 'rootId');
    if (projectId === null || rootId === null) {
      return { kind: 'failed', error: badPayload('install payload') };
    }

    try {
      // Two opaque ids and nothing else. §24.3a: the destination is composed in the core from
      // the root's own path and the project's stored `seed_basename`, so no path crosses here
      // in either direction and the renderer never assembles one.
      const start = await deps.request('install.start', { projectId, rootId });
      // A refusal is inside `start`, not thrown: §24.3d makes a collision a reply.
      return { kind: 'started', start };
    } catch (err: unknown) {
      return { kind: 'failed', error: asBridgeError(err) };
    }
  });

  deps.handle(IPC_INSTALL_CANCEL, async (payload: unknown): Promise<InstallCancelReply> => {
    const runId = asId(payload, 'runId');
    if (runId === null) return { kind: 'failed', error: badPayload('runId') };

    try {
      await deps.request('install.cancel', { runId });
      return { kind: 'cancelled' };
    } catch (err: unknown) {
      return { kind: 'failed', error: asBridgeError(err) };
    }
  });
}
