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

/**
 * Where the resolved tier came from (§11.2a's override chain). It lives here rather than
 * beside the resolver in `src/main/bootStore.ts` because `CodothecaBridge` carries it to the
 * renderer, and `tsconfig.web.json` excludes `src/main`.
 */
export type EffectsTierSource = 'argv' | 'environment' | 'paint-failure' | 'boot-file';

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

/**
 * The same two facts, carried to the preload the same way the tier is (§11.2a: no round trip,
 * because the core joins after first paint). Neither is an override the shell reads back —
 * `bootStore` resolves both — so there is no argv *input* parser for them, only this pair.
 */
export const EFFECTS_TIER_SOURCE_FLAG = '--effects-tier-source=';
export const PAINT_FAIL_FORCED_AT_FLAG = '--paint-fail-forced-at=';

/**
 * §11.3a's reduced-motion override, carried the same way for the same reason: it clamps the tier,
 * so a first frame that knew the tier and not the override would run motion the user turned down.
 * Present means on. Nothing overrides it from argv or the environment; only `boot.json` sets it.
 */
export const REDUCED_MOTION_OVERRIDE_FLAG = '--reduced-motion-override';

export function reducedMotionOverrideFromArgv(argv: readonly string[]): boolean {
  return argv.includes(REDUCED_MOTION_OVERRIDE_FLAG);
}

const EFFECTS_TIER_SOURCES = [
  'argv',
  'environment',
  'paint-failure',
  'boot-file',
] as const satisfies readonly EffectsTierSource[];

/** The last parseable `--effects-tier-source=` in `argv`, or `null`. */
export function effectsTierSourceFromArgv(argv: readonly string[]): EffectsTierSource | null {
  let found: EffectsTierSource | null = null;
  for (const argument of argv) {
    if (argument.startsWith(EFFECTS_TIER_SOURCE_FLAG)) {
      const value = argument.slice(EFFECTS_TIER_SOURCE_FLAG.length).trim();
      if ((EFFECTS_TIER_SOURCES as readonly string[]).includes(value)) {
        found = value as EffectsTierSource;
      }
    }
  }
  return found;
}

/**
 * The last parseable `--paint-fail-forced-at=` in `argv`, or `null`. `null` is "no launch
 * forced the tier off", so a zero or an unparseable value reads as absent rather than as a
 * forcing at the epoch.
 */
export function paintFailForcedAtFromArgv(argv: readonly string[]): number | null {
  let found: number | null = null;
  for (const argument of argv) {
    if (argument.startsWith(PAINT_FAIL_FORCED_AT_FLAG)) {
      const value = Number(argument.slice(PAINT_FAIL_FORCED_AT_FLAG.length).trim());
      found = Number.isFinite(value) && value > 0 ? value : null;
    }
  }
  return found;
}
