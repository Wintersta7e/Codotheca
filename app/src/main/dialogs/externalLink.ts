/**
 * The shell half of §25.2's external opener (A9). It is `dialogs/relocate.ts`'s shape with a URL
 * where the folder dialog was: an opaque id in, the privileged thing done in the main process, a
 * discriminated reply out.
 *
 * Three rules, and each one is the reason the file exists:
 *
 * 1. **No URL crosses IPC inbound.** The handler reads `projectId` and `kind` and nothing else,
 *    so a renderer that sends a `url` field changes nothing — exactly as `relocate.ts` records
 *    for `pathBytes`.
 * 2. **The answer is re-asserted here, not trusted.** Scheme, userinfo, port, query, fragment,
 *    path shape and host are all checked again on the string the core returned. One process
 *    building a URL and another opening it without looking is a single point of failure.
 * 3. **`denyPermissionRequest` is untouched.** `openExternal` stays a denied permission name:
 *    that handler grants for a whole session, and the renderer has no business holding the
 *    capability. The opener is a channel, not a grant.
 */
import type { Account } from '../../generated/protocol';
import {
  type BridgeError,
  IPC_OPEN_REMOTE_LINK,
  type OpenRemoteLinkReply,
} from '../../shared/channels';
import type { BridgeRequest } from '../core/bridge';

/**
 * The base allowlist, mirrored from `ALLOWLIST_BASE` in `core/src/remote/weburl.rs`.
 *
 * It decides whether a URL opens, so it gets R24's remedy rather than a comment:
 * `app/test/remoteAllowlist.test.ts` reads the Rust source and asserts the two are equal,
 * failing first if it read nothing.
 */
export const REMOTE_HOST_ALLOWLIST: readonly string[] = ['github.com'];

/**
 * The path suffixes §25.2 admits, and the whole of what a returned path may carry beyond
 * `/<owner>/<name>`. `repository` adds none.
 */
const LINK_SUFFIXES: readonly string[] = ['issues', 'pulls', 'actions', 'releases'];

export interface ExternalLinkDeps {
  handle: (channel: string, fn: (payload: unknown) => Promise<OpenRemoteLinkReply>) => void;
  request: BridgeRequest;
  /** The per-click confirmation. It is handed the **whole** URL, and it names it. */
  confirm: (url: string) => Promise<boolean>;
  openExternal: (url: string) => Promise<void>;
}

interface LinkCall {
  readonly projectId: number;
  readonly kind: string;
}

/**
 * Exactly two fields, read by name. Nothing else on the payload is looked at, which is what
 * makes "no URL crosses IPC" a property of the code rather than of the caller.
 *
 * `kind` is checked for shape and not against a vocabulary: the schema declares
 * `RemoteLinkKind` and R31 forbids a second hand-written copy of it, so an unknown kind is
 * refused by the core with `PROTOCOL` and arrives here as a failure.
 */
function asLinkCall(payload: unknown): LinkCall | null {
  if (typeof payload !== 'object' || payload === null) return null;
  const record = payload as Record<string, unknown>;
  const projectId = record['projectId'];
  const kind = record['kind'];
  if (typeof projectId !== 'number' || !Number.isSafeInteger(projectId) || projectId <= 0) {
    return null;
  }
  if (typeof kind !== 'string' || kind.length === 0) return null;
  return { projectId, kind };
}

/**
 * The Enterprise half of the allowlist, read from the **account row** and from nowhere else.
 *
 * One value with two readers is fine; one value with two *sources* is R12. The core reads
 * `account.host` out of SQLite and the shell reads the same column through `accounts.list`, so
 * neither side holds a host literal and a host that was never connected is never accepted.
 */
async function connectedHosts(request: BridgeRequest): Promise<string[]> {
  try {
    const accounts = (await request('accounts.list', {})) as readonly Account[];
    return accounts.map((account) => account.host);
  } catch {
    // A failed read is not evidence that a host is absent — but it is also not permission to
    // open a link this process cannot justify. The base allowlist still applies, so github.com
    // keeps working and an Enterprise link is refused until the read succeeds.
    return [];
  }
}

/**
 * The second assertion on a URL the core already built. Anything unusual is refused rather than
 * normalised: this string is about to be handed to the operating system.
 */
export function isOpenableUrl(candidate: string, allowedHosts: readonly string[]): boolean {
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    return false;
  }
  if (url.protocol !== 'https:') return false;
  if (url.username !== '' || url.password !== '') return false;
  if (url.port !== '') return false;
  if (url.search !== '' || url.hash !== '') return false;
  if (!allowedHosts.includes(url.hostname)) return false;

  const segments = url.pathname.split('/').filter((segment) => segment !== '');
  if (segments.length === 2) return true;
  if (segments.length !== 3) return false;
  const suffix = segments[2];
  return suffix !== undefined && LINK_SUFFIXES.includes(suffix);
}

/**
 * §2.2: `outcome` is `'unknown'` or absent. There is no `'failed'` member — a refusal that
 * definitely did not take effect carries `null`.
 */
function asBridgeError(err: unknown): BridgeError {
  const e = typeof err === 'object' && err !== null ? (err as Record<string, unknown>) : null;
  const code = e !== null && typeof e['code'] === 'string' ? e['code'] : 'INTERNAL';
  const message =
    e !== null && typeof e['message'] === 'string' ? e['message'] : 'the link could not be opened';
  return {
    code: code as BridgeError['code'],
    message,
    outcome: e !== null && e['outcome'] === 'unknown' ? 'unknown' : null,
    retryable: e !== null && e['retryable'] === true,
  };
}

export function registerExternalLink(deps: ExternalLinkDeps): void {
  deps.handle(IPC_OPEN_REMOTE_LINK, async (payload: unknown): Promise<OpenRemoteLinkReply> => {
    const call = asLinkCall(payload);
    if (call === null) {
      return {
        kind: 'failed',
        error: {
          code: 'PROTOCOL',
          message: 'open-remote-link: bad payload',
          outcome: null,
          retryable: false,
        },
      };
    }

    let url: unknown;
    try {
      url = await deps.request('remote.webUrl', { projectId: call.projectId, kind: call.kind });
    } catch (err: unknown) {
      return { kind: 'failed', error: asBridgeError(err) };
    }
    // NULL is the core saying this project produces no link. It draws nothing and is not an
    // error — the links row is already absent on that surface.
    if (url === null || url === undefined) return { kind: 'not_linkable' };
    if (typeof url !== 'string') return { kind: 'not_linkable' };

    const allowed = [...REMOTE_HOST_ALLOWLIST, ...(await connectedHosts(deps.request))];
    if (!isOpenableUrl(url, allowed)) return { kind: 'not_linkable' };

    // Per click. No "always allow", no settings suppression, no memory of a previous answer.
    if (!(await deps.confirm(url))) return { kind: 'declined' };

    try {
      await deps.openExternal(url);
    } catch (err: unknown) {
      return { kind: 'failed', error: asBridgeError(err) };
    }
    return { kind: 'opened', url };
  });
}
