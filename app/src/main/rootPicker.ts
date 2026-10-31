/**
 * The shell half of adding a scan root (§2.4, §10.1b).
 *
 * §2.4 makes `roots.add` privileged and forbids the renderer from originating a filesystem
 * path, so a folder can only reach the core through a native dialog this process owns. This is
 * that dialog, and R11 makes it the only `ipcMain.handle` on `IPC_PICK_ROOT` in the product:
 * Electron throws at a second registration, so a duplicate is a startup crash rather than a
 * duplicated string.
 */
import type { RootAdd, RootsAddArgs, RootSuggestion } from '../generated/protocol';
import {
  type BridgeError,
  type CommitSuggestionReply,
  type CommitSuggestionRequest,
  IPC_COMMIT_SUGGESTION,
  IPC_PICK_ROOT,
  type PickRootReply,
} from '../shared/channels';

/**
 * §2.5: bytes that must cross are tagged, never sent as a display string.
 *
 * `dialogs/relocate.ts` has `pathToBytes` doing the same job for the other dialog that owns a
 * path. One of the two should become the shell's single byte-tagging helper; neither plan
 * declares an owner for it, so this one is left where its plan puts it and the overlap is
 * reported rather than resolved across a plan boundary.
 */
export function encodePathBytes(path: string): { readonly b64: string } {
  return { b64: Buffer.from(path, 'utf8').toString('base64') };
}

export interface RootPickerDeps {
  readonly showOpenDialog: () => Promise<{ canceled: boolean; filePaths: string[] }>;
  readonly addRoot: (args: RootsAddArgs) => Promise<RootAdd>;
  readonly handle: (channel: string, fn: (payload: unknown) => Promise<PickRootReply>) => void;
}

function isRequest(value: unknown): value is { confirmLarge: boolean } {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { confirmLarge?: unknown }).confirmLarge === 'boolean'
  );
}

/**
 * §2.2: `outcome` is `'unknown'` or absent, and there is no `'failed'` member. A payload
 * rejected before the dialog opened definitely did not take effect, so it carries `null`.
 */
const PROTOCOL_ERROR: BridgeError = {
  code: 'PROTOCOL',
  message: 'pick-root: confirmLarge must be a boolean',
  outcome: null,
  retryable: false,
};

export function registerRootPicker(deps: RootPickerDeps): void {
  deps.handle(IPC_PICK_ROOT, async (payload): Promise<PickRootReply> => {
    if (!isRequest(payload)) {
      return { kind: 'failed', error: PROTOCOL_ERROR };
    }
    const chosen = await deps.showOpenDialog();
    const first = chosen.filePaths[0];
    if (chosen.canceled || first === undefined) {
      return { kind: 'cancelled' };
    }
    try {
      const add = await deps.addRoot({
        pathBytes: encodePathBytes(first),
        confirmLarge: payload.confirmLarge,
        // A folder from this dialog is a native path; only a suggestion the core made names a
        // WSL distro.
        distro: '',
      });
      // A refusal is a reply, not a failure: §10.1a requires an explanation and never a
      // silent no, and the screen draws refusals as rows.
      return { kind: 'added', add };
    } catch (error) {
      const code = (error as { code?: unknown }).code;
      return {
        kind: 'failed',
        error: {
          code: typeof code === 'string' ? (code as BridgeError['code']) : 'CORE_RESTARTED',
          message: error instanceof Error ? error.message : 'the core did not answer',
          // The request left this process. §2.2 forbids auto-replaying it.
          outcome: 'unknown',
          retryable: false,
        },
      };
    }
  });
}

/**
 * What the last `roots.suggest` proposed, keyed by what the renderer can see of a row: its
 * `kind`, `distro` and `pathDisplay`.
 *
 * GAP-16b-1. §2.4 forbids the renderer from originating a filesystem path, so a ticked
 * suggestion arrives here as display parts and nothing else. The cache is what turns them back
 * into a folder **the core itself proposed**: `resolve` answers only for parts exactly one
 * remembered suggestion carries. The path alone is not enough — every WSL distro row renders the
 * same home-parent path, so keying on it refused two ticked distros as ambiguous.
 *
 * `null` for unknown parts and `null` for ambiguous ones, and the caller keeps them apart. Two
 * conventional locations can render the same string, and adding the wrong folder is worse than
 * adding none.
 */
export class SuggestionCache {
  private rows: readonly RootSuggestion[] = [];

  remember(rows: readonly RootSuggestion[]): void {
    this.rows = rows;
  }

  /** How many remembered suggestions carry these display parts: 0, 1, or more. */
  count(key: CommitSuggestionRequest): number {
    return this.hits(key).length;
  }

  /**
   * The remembered suggestion for `key`, or `null` when no suggestion carries it or more than
   * one does.
   *
   * The value returned is the **cache's** row, never the caller's: what reaches `roots.add` —
   * the path and the distro — is what the core sent, not what the renderer typed.
   */
  resolve(key: CommitSuggestionRequest): RootSuggestion | null {
    const hits = this.hits(key);
    return hits.length === 1 ? (hits[0] ?? null) : null;
  }

  private hits(key: CommitSuggestionRequest): readonly RootSuggestion[] {
    return this.rows.filter(
      (row) =>
        row.kind === key.kind && row.distro === key.distro && row.pathDisplay === key.pathDisplay,
    );
  }
}

export interface SuggestionCommitDeps {
  readonly suggestRoots: () => Promise<readonly RootSuggestion[]>;
  readonly addRoot: (args: RootsAddArgs) => Promise<RootAdd>;
  readonly handle: (
    channel: string,
    fn: (payload: unknown) => Promise<CommitSuggestionReply>,
  ) => void;
  /** Shared so a test can seed it; the handler refreshes it on every commit. */
  readonly cache: SuggestionCache;
}

function isCommitRequest(value: unknown): value is CommitSuggestionRequest {
  if (typeof value !== 'object' || value === null) return false;
  const v = value as { kind?: unknown; distro?: unknown; pathDisplay?: unknown };
  return (
    typeof v.kind === 'string' && typeof v.distro === 'string' && typeof v.pathDisplay === 'string'
  );
}

/**
 * The other half of §10.1b: `DIG IN` with only *suggested* roots ticked used to add nothing at
 * all, because `commitSuggestion` resolved against a cache that existed in no file.
 *
 * The suggestions are re-read on every commit rather than trusted from when the screen was
 * drawn. A folder that has since gone is then refused instead of added, which is the direction
 * that cannot be wrong.
 *
 * `confirmLarge` is **true**: §10.1b permits the directory estimate only on a folder the user
 * picked in a dialog, never on a row the core proposed. The core's own `SuggestionCache` says
 * the same thing from the other side.
 */
export function registerSuggestionCommit(deps: SuggestionCommitDeps): void {
  deps.handle(IPC_COMMIT_SUGGESTION, async (payload): Promise<CommitSuggestionReply> => {
    if (!isCommitRequest(payload)) {
      return { kind: 'failed', error: PROTOCOL_ERROR_COMMIT };
    }
    let rows: readonly RootSuggestion[];
    try {
      rows = await deps.suggestRoots();
    } catch (error) {
      return { kind: 'failed', error: coreError(error) };
    }
    deps.cache.remember(rows);

    const resolved = deps.cache.resolve(payload);
    if (resolved === null) {
      return deps.cache.count(payload) > 1 ? { kind: 'ambiguous' } : { kind: 'unknown' };
    }
    try {
      const add = await deps.addRoot({
        pathBytes: encodePathBytes(resolved.pathDisplay),
        confirmLarge: true,
        distro: resolved.distro,
      });
      return { kind: 'added', add };
    } catch (error) {
      return { kind: 'failed', error: coreError(error) };
    }
  });
}

const PROTOCOL_ERROR_COMMIT: BridgeError = {
  code: 'PROTOCOL',
  message: 'commit-suggestion: kind, distro and pathDisplay must be strings',
  outcome: null,
  retryable: false,
};

/** §2.2: a request that left this process may have completed, so it is never auto-replayed. */
function coreError(error: unknown): BridgeError {
  const code = (error as { code?: unknown }).code;
  return {
    code: typeof code === 'string' ? (code as BridgeError['code']) : 'CORE_RESTARTED',
    message: error instanceof Error ? error.message : 'the core did not answer',
    outcome: 'unknown',
    retryable: false,
  };
}
