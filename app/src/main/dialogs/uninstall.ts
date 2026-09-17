/**
 * The shell half of §24.8's removal.
 *
 * **There is no dialog here, and that is the point.** `IPC_RELOCATE` exists because a path must be
 * originated by the shell; this channel originates nothing — the renderer supplies a `locationId`
 * and the core reads the path from the row it is about to re-verify. What this channel is *for* is
 * the refusal: `locations.uninstall` is privileged, so `isRendererCallable` keeps it off the
 * renderer-callable surface, and this is the only route it has instead.
 *
 * **The diagnostic strings deliberately avoid the word.** The direct translations of
 * `relocate.ts`'s equivalents both match `\buninstall\b` — measured — and would force a fourth
 * entry into §24.2c's site list for strings no user ever reads. `IPC_UNINSTALL`'s *value* is a
 * different matter: a channel has to say what it is, so `channels.ts` is an enumerated site.
 */
import { type BridgeError, IPC_UNINSTALL, type UninstallReply } from '../../shared/channels';
import type { BridgeRequest } from '../core/bridge';

export interface UninstallDeps {
  handle: (channel: string, fn: (payload: unknown) => Promise<UninstallReply>) => void;
  request: BridgeRequest;
}

/** The same validation `relocate.ts` applies, for the same reason: nothing else is passed on. */
function asLocationId(payload: unknown): number | null {
  if (typeof payload !== 'object' || payload === null) return null;
  const id = (payload as Record<string, unknown>)['locationId'];
  return typeof id === 'number' && Number.isSafeInteger(id) && id > 0 ? id : null;
}

/**
 * §2.2: `outcome` is `'unknown'` or absent. A refusal that definitely did not take effect carries
 * `null` — treating that as a word would tell the renderer a removal may have landed when the core
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

export function registerUninstall(deps: UninstallDeps): void {
  deps.handle(IPC_UNINSTALL, async (payload: unknown): Promise<UninstallReply> => {
    const locationId = asLocationId(payload);
    if (locationId === null) {
      return {
        kind: 'failed',
        error: {
          code: 'PROTOCOL',
          message: 'bad locationId',
          outcome: null,
          retryable: false,
        },
      };
    }

    try {
      // A `locationId` and nothing else. No verdict token crosses this boundary, so there is
      // nothing here for a caller to forge or replay.
      const location = await deps.request('locations.uninstall', { locationId });
      return { kind: 'uninstalled', location };
    } catch (err: unknown) {
      const error = asBridgeError(err);
      // A recomputed verdict that is not `safe` comes back as a PROTOCOL refusal, which is a
      // **reply and not a failure**: nothing was removed, and the user can be told why.
      if (error.code === 'PROTOCOL') {
        return { kind: 'refused', verdict: error.message };
      }
      return { kind: 'failed', error };
    }
  });
}
