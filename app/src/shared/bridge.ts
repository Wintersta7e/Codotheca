import type { PickRootReply, RelocateReply, RevealTarget, ShortcutState } from './channels';
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
