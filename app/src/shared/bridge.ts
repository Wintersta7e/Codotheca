import type { RelocateReply } from './channels';
import type { EffectsTier } from './effectsTier';

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
  onCoreStatus(cb: (status: unknown) => void): void;
  /** One batch per frame, never one message per event. */
  onCoreEvents(cb: (batch: unknown) => void): void;
}
