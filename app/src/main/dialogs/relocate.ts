/**
 * The shell half of RELOCATE (§8.5.2, §17). It rewrites one `location` row's path and nothing
 * else — no file is moved, deleted, created or opened for writing, and no git command mutates
 * anything.
 *
 * The renderer sends a LocationId and nothing else. The path comes from a native dialog this
 * process owns, which is the whole of §2.4's trust rule: a renderer-supplied string and a
 * dialog result are different trust categories, and only one of them may become a path.
 */
import type { Bytes } from '../../generated/protocol';
import { type BridgeError, IPC_RELOCATE, type RelocateReply } from '../../shared/channels';
import type { BridgeRequest } from '../core/bridge';

export interface RelocateDeps {
  handle: (channel: string, fn: (payload: unknown) => Promise<RelocateReply>) => void;
  /** Resolves to the chosen directory, or null when the user cancelled. */
  showFolderDialog: () => Promise<string | null>;
  request: BridgeRequest;
}

/** §2.5: where raw bytes cross the wire they are tagged, never stringified. */
export function pathToBytes(path: string): Bytes {
  return { b64: Buffer.from(path, 'utf8').toString('base64') };
}

function asLocationId(payload: unknown): number | null {
  if (typeof payload !== 'object' || payload === null) return null;
  const id = (payload as Record<string, unknown>)['locationId'];
  return typeof id === 'number' && Number.isSafeInteger(id) && id > 0 ? id : null;
}

/**
 * §2.2: `outcome` is `'unknown'` or absent. There is no `'failed'` member — a refusal that
 * definitely did not take effect carries `null`, and treating that as a word would tell the
 * renderer a write may have landed when the core has just said it did not.
 */
function asBridgeError(err: unknown): BridgeError {
  const e = typeof err === 'object' && err !== null ? (err as Record<string, unknown>) : null;
  const code = e !== null && typeof e['code'] === 'string' ? e['code'] : 'INTERNAL';
  const message = e !== null && typeof e['message'] === 'string' ? e['message'] : 'relocate failed';
  return {
    code: code as BridgeError['code'],
    message,
    outcome: e !== null && e['outcome'] === 'unknown' ? 'unknown' : null,
    retryable: e !== null && e['retryable'] === true,
  };
}

export function registerRelocateDialog(deps: RelocateDeps): void {
  deps.handle(IPC_RELOCATE, async (payload: unknown): Promise<RelocateReply> => {
    const locationId = asLocationId(payload);
    if (locationId === null) {
      return {
        kind: 'failed',
        error: {
          code: 'PROTOCOL',
          message: 'relocate: bad locationId',
          outcome: null,
          retryable: false,
        },
      };
    }

    const chosen = await deps.showFolderDialog();
    if (chosen === null) return { kind: 'cancelled' };

    try {
      // Only the dialog's answer becomes `pathBytes`. A `pathBytes` in the renderer's payload is
      // read by nothing here, so a renderer that sends one changes nothing.
      const location = await deps.request('locations.relocate', {
        locationId,
        pathBytes: pathToBytes(chosen),
      });
      return { kind: 'relocated', location };
    } catch (err: unknown) {
      return { kind: 'failed', error: asBridgeError(err) };
    }
  });
}
