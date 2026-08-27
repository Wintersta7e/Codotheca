/**
 * The effects tier (§11.3), settable before the window renders because the GPU is what may be
 * broken.
 *
 * `auto` is a *resolver*, not a stored state — §11.6 resolves it in the renderer against
 * `prefers-reduced-motion`, the compositor and WebGL context loss. Nothing here resolves it;
 * this module only carries the value between the boot file, argv, the environment and the
 * renderer.
 *
 * R31: `EffectsTier` itself is declared in `protocol/schema/protocol.json` and generated. The
 * generated type is the only one; this module re-exports it so the tier and its parser stay
 * one import for every consumer. `satisfies` keeps `EFFECTS_TIERS` a literal tuple while
 * making any entry that is not a schema variant a type error.
 */
import type { EffectsTier } from '../generated/protocol';

export type { EffectsTier };

export const EFFECTS_TIERS = [
  'auto',
  'full',
  'reduced',
  'off',
] as const satisfies readonly EffectsTier[];

/** §11.2a's argv override, and the same string the renderer receives via additionalArguments. */
export const EFFECTS_TIER_FLAG = '--effects-tier=';

/** §11.2a's environment override. */
export const EFFECTS_TIER_ENV_VAR = 'CODOTHECA_EFFECTS_TIER';

function isEffectsTier(value: string): value is EffectsTier {
  return (EFFECTS_TIERS as readonly string[]).includes(value);
}

/** `null` means "not set", which is not the same as "set to auto". */
export function parseEffectsTier(value: string | undefined): EffectsTier | null {
  if (value === undefined) {
    return null;
  }
  const normalised = value.trim().toLowerCase();
  return isEffectsTier(normalised) ? normalised : null;
}

/** The last parseable `--effects-tier=` in `argv`, or `null`. */
export function effectsTierFromArgv(argv: readonly string[]): EffectsTier | null {
  let found: EffectsTier | null = null;
  for (const argument of argv) {
    if (argument.startsWith(EFFECTS_TIER_FLAG)) {
      const parsed = parseEffectsTier(argument.slice(EFFECTS_TIER_FLAG.length));
      if (parsed !== null) {
        found = parsed;
      }
    }
  }
  return found;
}
