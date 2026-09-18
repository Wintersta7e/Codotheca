/**
 * The IPC surface between the shell and the renderer. Imported by both ends, so a channel
 * name cannot drift between them.
 */
import type { ErrorCode, Outcome, RemoteLinkKind, RootAdd, Topic } from '../generated/protocol';

export const IPC_REQUEST = 'codotheca:request';
export const IPC_CORE_STATUS = 'codotheca:core-status';
export const IPC_EVENTS = 'codotheca:events';

/**
 * The shell tells the renderer to open quick switch. §8.6: pressing the resident show shortcut
 * shows the window and opens the palette; tray-icon activation shows the window and opens
 * nothing, so the tray path never sends on this channel.
 */
export const IPC_OPEN_PALETTE = 'codotheca:open-palette';

// R32: the channel and the frame live beside the class that publishes them. Plans 17 and 17b
// import both and declare neither; `src/shared` because the renderer cannot import `src/main`.
export const IPC_SHORTCUT_STATE = 'codotheca:shortcut-state';

/** §8.6: a chord that failed to register reads back with the chord and `registered: false`. */
export interface ShortcutState {
  readonly chord: string | null;
  readonly registered: boolean;
}

export interface RendererEvent {
  topic: Topic;
  event: string;
  data: unknown;
}

export interface BridgeError {
  code: ErrorCode;
  message: string;
  /** `null` — definitely did not take effect. `'unknown'` — may have (§2.2). */
  outcome: Outcome | null;
  retryable: boolean;
}

export type BridgeReply = { ok: true; value: unknown } | { ok: false; error: BridgeError };

export interface BridgeCall {
  name: string;
  args: unknown;
}

/**
 * §2.4: `locations.relocate` is privileged, so `isRendererCallable` refuses it on IPC_REQUEST.
 * It travels this channel instead, where the shell owns the native folder dialog and the
 * renderer supplies nothing but an opaque LocationId. A renderer-supplied string and a dialog
 * result are different trust categories, and only one of them may become a path.
 */
export const IPC_RELOCATE = 'codotheca:relocate';

export interface RelocateCall {
  locationId: number;
}

/**
 * §24.8's reachability rule, on `IPC_RELOCATE`'s shape. `install.start` mutates the filesystem,
 * so it is privileged and `isRendererCallable` refuses it on IPC_REQUEST; it travels here
 * instead. The renderer supplies an opaque ProjectId and an opaque RootId and **no path in
 * either direction** — §24.3a makes the destination a root the user already added, and a folder
 * that is not yet one is reached through `IPC_PICK_ROOT`, which stays the only path-origination
 * channel the product has.
 */
export const IPC_INSTALL_START = 'codotheca:install-start';

export interface InstallStartCall {
  readonly projectId: number;
  readonly rootId: number;
}

/**
 * `install.start` answers with an `InstallStart`, which carries either a run id **or** a refusal
 * — §24.3d's *"a refusal is a reply, not a failure"*. So there is no `refused` member here: the
 * refusal rides inside `start` and only a transport or core error becomes `failed`.
 */
export type InstallStartReply =
  | { readonly kind: 'started'; readonly start: unknown }
  | { readonly kind: 'failed'; readonly error: BridgeError };

/**
 * §24.3c: cancel kills the clone's process group and removes the staging directory, so it is
 * privileged for the same reason the start is and takes the same route. It names a run and
 * nothing else — there is no path to send and none to accept back.
 */
export const IPC_INSTALL_CANCEL = 'codotheca:install-cancel';

export interface InstallCancelCall {
  readonly runId: number;
}

/** `install.cancel` returns `Empty`, so the only thing to carry back is whether it threw. */
export type InstallCancelReply =
  { readonly kind: 'cancelled' } | { readonly kind: 'failed'; readonly error: BridgeError };

/**
 * §24.8's removal, on `IPC_RELOCATE`'s shape — **and it has no dialog, which is the difference
 * worth stating.** `IPC_RELOCATE` exists because a path must be originated by the shell; this
 * channel originates nothing. The renderer supplies a `locationId` and the core reads the path
 * from the row it is about to re-verify.
 *
 * The channel's job here is the **refusal**: `locations.uninstall` is privileged, so
 * `isRendererCallable` keeps it off the renderer-callable surface, which is what §24.8's
 * reachability row asks for. `locations.uninstallPreflight` is unprivileged and rides
 * `IPC_REQUEST` normally — it carries no `Bytes` and mutates nothing.
 */
export const IPC_UNINSTALL = 'codotheca:uninstall';

export interface UninstallCall {
  readonly locationId: number;
}

export type UninstallReply =
  | { readonly kind: 'uninstalled'; readonly location: unknown }
  /** The core recomputed the verdict and it was not `safe`. Not a failure — a reply. */
  | { readonly kind: 'refused'; readonly verdict: unknown }
  | { readonly kind: 'failed'; readonly error: BridgeError };

/**
 * §25.2's external opener, and it is `IPC_RELOCATE`'s shape with a URL where the folder dialog
 * was. The renderer sends an opaque project id and a link kind; **no URL crosses this channel
 * inbound**, because `remote_key` is derived from repository content the user may not have
 * written and a relayed URL is hostile input aimed at the process that owns the dialogs.
 *
 * The URL the shell opens is the one the core built from the stored key, re-asserted here
 * against the same host allowlist, and confirmed by the user **per click**.
 */
export const IPC_OPEN_REMOTE_LINK = 'codotheca:open-remote-link';

export interface OpenRemoteLinkCall {
  readonly projectId: number;
  readonly kind: RemoteLinkKind;
}

/**
 * `not_linkable` is not a failure: it is the core answering that this project produces no link,
 * which is what a non-allowlisted host, a NULL `remote_key` and a key that is not
 * `<host>/<owner>/<name>` all mean. `declined` is the user saying no to the confirmation.
 */
/**
 * `opened` carries **no URL**. §25.2's rule is that the URL is reconstructed by the core and
 * never passed through, and a reply is still a crossing: it put the address the shell built back
 * into the sandboxed process, in the one direction the rule does not spell out, for a field no
 * caller reads. The renderer names a project and a kind; what that resolves to is not its
 * business in either direction.
 */
export type OpenRemoteLinkReply =
  | { readonly kind: 'opened' }
  | { readonly kind: 'declined' }
  | { readonly kind: 'not_linkable' }
  | { readonly kind: 'failed'; readonly error: BridgeError };

// R11: `IPC_PICK_ROOT` and `PickRootReply` are declared in this file by plan 16, whose
// first-run flow picks the first root (§10.1b) and owns the only `ipcMain.handle` for that
// channel. §11's settings drawer calls through it and declares neither.
export const IPC_PICK_EXECUTABLE = 'codotheca:pick-executable';
export const IPC_REVEAL = 'codotheca:reveal';
export const IPC_INDEX_LOCATION = 'codotheca:index-location';
export const IPC_CLEAR_PAINT_FAILURE = 'codotheca:clear-paint-failure';

/**
 * The only three things the shell will reveal. A renderer-supplied path is refused (§2.4): the
 * renderer names a target, never a location.
 *
 * `log` is named because §11.1's degraded notice and §11.2a's failure windows both offer
 * `OPEN THE LOG`, and the rolling log is not inside the index or the bundle.
 */
export type RevealTarget = 'index' | 'bundle' | 'log';

export interface IndexLocation {
  readonly pathDisplay: string;
  readonly sizeBytes: number;
}

/** Must equal `Index::db_path`'s file name; `app/test/shellServices.test.ts` reads it there. */
export const INDEX_DB_FILE = 'index.db';

export type RelocateReply =
  | { kind: 'relocated'; location: unknown }
  | { kind: 'cancelled' }
  | { kind: 'failed'; error: BridgeError };

/**
 * The native folder dialog. §2.4: the renderer may never originate a filesystem path, and
 * `roots.add` is privileged, so a folder can only reach the core through a dialog the shell
 * owns. This is that dialog.
 *
 * R11: declared here once, and `registerRootPicker` is the sole `ipcMain.handle` on it —
 * Electron throws at a second registration, so a duplicate is a startup crash. Every other
 * surface that needs a folder imports this constant and invokes the channel.
 */
export const IPC_PICK_ROOT = 'codotheca:pick-root';

export interface PickRootRequest {
  readonly confirmLarge: boolean;
}

/**
 * Committing a **suggested** root — GAP-16b-1.
 *
 * §2.4 bars the renderer from originating a filesystem path and `RootSuggestion` carries no
 * bytes, so the renderer holds only a display string. The shell resolves that string against
 * what `roots.suggest` returned and calls `roots.add` with the path the core itself produced;
 * a string that names no suggestion is **refused**, never guessed.
 */
export const IPC_COMMIT_SUGGESTION = 'codotheca:commit-suggestion';

export interface CommitSuggestionRequest {
  readonly pathDisplay: string;
}

/**
 * `unknown` and `ambiguous` are refusals, not failures: nothing went wrong, and nothing was
 * added. Two suggestions can render one string, and adding the wrong folder is worse than
 * adding none.
 */
export type CommitSuggestionReply =
  | { readonly kind: 'added'; readonly add: RootAdd }
  | { readonly kind: 'unknown' }
  | { readonly kind: 'ambiguous' }
  | { readonly kind: 'failed'; readonly error: BridgeError };

export type PickRootReply =
  | { readonly kind: 'cancelled' }
  | { readonly kind: 'added'; readonly add: RootAdd }
  | { readonly kind: 'failed'; readonly error: BridgeError };
