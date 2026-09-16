import type { RemoteLinkKind } from '../generated/protocol';
import type {
  CommitSuggestionReply,
  OpenRemoteLinkReply,
  PickRootReply,
  RelocateReply,
  RevealTarget,
  ShortcutState,
} from './channels';
import type { EffectsTier, EffectsTierSource } from './effectsTier';

/** The single `contextBridge` key. Nothing else is exposed on `window`. */
export const CODOTHECA_BRIDGE_KEY = 'codotheca';

/**
 * Everything the sandboxed renderer may see. It carries no path and no executable: §2.4 puts
 * both behind a shell-owned native dialog.
 */
export interface CodothecaBridge {
  readonly protocolVersion: number;
  /**
   * The tier the shell resolved before the window was created. Still possibly `auto` — §11.6
   * resolves that in the renderer, where the media query and the compositor live.
   */
  readonly effectsTier: EffectsTier;
  /**
   * Which link in §11.2a's override chain decided that tier. §11.3's drawer states it, because
   * a tier the user did not choose and cannot account for reads as a broken control.
   */
  readonly effectsTierSource: EffectsTierSource;
  /**
   * When a launch forced the tier `off`. `null` is "no launch forced it" — never a forcing at
   * the epoch.
   */
  readonly paintFailForcedAt: number | null;
  /**
   * The rolling log's path, for §11.2a's failure windows to name.
   *
   * §2.4 is not bent by carrying it: this is a *display* string on a screen that exists because
   * something failed, never a path the renderer originates or acts on — the renderer cannot
   * open it, and the two things the shell will reveal are named targets, not paths.
   *
   * The empty string is "the shell passed none", which draws no note at all.
   */
  readonly logPath: string;
  /**
   * One command. Resolves to a `BridgeReply`, never rejects: the renderer needs `code`,
   * `outcome` and `retryable` to decide whether a retry is safe, and a thrown string carries
   * none of them.
   */
  request(name: string, args: unknown): Promise<unknown>;
  /**
   * The one privileged operation the renderer may ask for. It sends an opaque LocationId and
   * nothing else; the path comes from a native dialog the shell owns, because §2.4 forbids the
   * renderer from originating one.
   */
  relocate(locationId: number): Promise<RelocateReply>;
  /**
   * §25.2: open one of the five remote links. The renderer sends an opaque project id and a
   * link kind — **never a URL**. The shell asks the core for the string, re-asserts the host
   * allowlist on it, names the whole URL in a confirmation, and only then opens it.
   */
  openRemoteLink(projectId: number, kind: RemoteLinkKind): Promise<OpenRemoteLinkReply>;
  /**
   * The native folder dialog, and the only path by which a folder reaches `roots.add`.
   *
   * It carries **no path in either direction on the way in**: the renderer asks, the shell opens
   * the dialog, and the chosen folder never round-trips through the sandbox. §2.4 forbids the
   * renderer from originating a filesystem path, and `roots.add` is privileged, so this is the
   * whole of first run's and the drawer's `ADD A FOLDER`.
   *
   * `confirmLarge` is the caller having already accepted a large-directory estimate.
   */
  pickRoot(confirmLarge: boolean): Promise<PickRootReply>;
  /**
   * Commit a **suggested** root — GAP-16b-1.
   *
   * The renderer holds only the display string `roots.suggest` returned; the shell resolves it
   * against that same list and calls `roots.add` with the path the core produced. A string that
   * names no suggestion, or two, is refused rather than guessed.
   */
  commitSuggestion(pathDisplay: string): Promise<CommitSuggestionReply>;
  onCoreStatus(cb: (status: unknown) => void): void;
  /** One batch per frame, never one message per event. */
  onCoreEvents(cb: (batch: unknown) => void): void;
  /**
   * §8.6: the resident show shortcut shows the window and opens the palette. The renderer is
   * told to open it; it never learns the chord, and no path here carries one.
   */
  onOpenPalette(cb: () => void): void;
  /**
   * The native executable dialog (§2.4). The renderer names a scope; the bytes never leave
   * the shell except as a `targets.upsert` it did not compose.
   */
  pickExecutable(scope: unknown): Promise<unknown>;
  /** Two named targets, never a path. */
  reveal(target: RevealTarget): Promise<unknown>;
  indexLocation(): Promise<unknown>;
  /** §11.3: reachable from settings, because a user who cannot see the window is stuck. */
  clearPaintFailure(): Promise<unknown>;
  /** [R32] Plan 15 owns the binding and is the only thing that publishes its state. */
  onShortcutState(cb: (state: ShortcutState) => void): void;
}
