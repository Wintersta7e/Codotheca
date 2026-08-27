import type { EffectsTier } from './effectsTier';

/** The single `contextBridge` key. Nothing else is exposed on `window`. */
export const CODOTHECA_BRIDGE_KEY = 'codotheca';

/**
 * Everything the sandboxed renderer may see. It carries no path and no executable: §2.4 puts
 * both behind a shell-owned native dialog. Plan 03 adds the protocol client here.
 */
export interface CodothecaBridge {
  readonly protocolVersion: number;
  /**
   * The tier the shell resolved before the window was created. Still possibly `auto` — §11.6
   * resolves that in the renderer, where the media query and the compositor live.
   */
  readonly effectsTier: EffectsTier;
}
