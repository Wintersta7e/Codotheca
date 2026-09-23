import type { CustomScheme } from 'electron';
import type { BootFile } from '../shared/bootFile';
import {
  EFFECTS_TIER_FLAG,
  EFFECTS_TIER_SOURCE_FLAG,
  type EffectsTier,
  PAINT_FAIL_FORCED_AT_FLAG,
  REDUCED_MOTION_OVERRIDE_FLAG,
} from '../shared/effectsTier';
import { type EffectsTierSource, resolveBootEffectsTier } from './bootStore';
import { registerArtSchemePrivileges } from './scheme';

/**
 * Everything that must happen before `app.ready`, in order, with every effect injected.
 *
 * The order is the point: it is invisible in `index.ts` and Electron only complains about
 * getting it wrong at runtime, so it is asserted by a test instead.
 */
export interface BootstrapDeps {
  readonly argv: readonly string[];
  readonly env: NodeJS.ProcessEnv;
  readonly userDataDir: string;
  readonly registerSchemesAsPrivileged: (schemes: CustomScheme[]) => void;
  readonly disableHardwareAcceleration: () => void;
  readonly readBoot: (userDataDir: string) => BootFile;
  readonly writeBoot: (userDataDir: string, file: BootFile) => void;
}

export interface BootstrapResult {
  readonly tier: EffectsTier;
  readonly source: EffectsTierSource;
  readonly stored: BootFile;
}

export function bootstrap(deps: BootstrapDeps): BootstrapResult {
  registerArtSchemePrivileges(deps.registerSchemesAsPrivileged);

  const stored = deps.readBoot(deps.userDataDir);
  const { tier, source } = resolveBootEffectsTier({
    argv: deps.argv,
    env: deps.env,
    stored,
  });

  if (tier === 'off') {
    // The GPU is what may be broken, so the recovery path must not require it.
    deps.disableHardwareAcceleration();
  }

  // Incremented before the window is created; cleared by clearPaintFailure on the first
  // composited frame. An override is never written back (§11.2a).
  deps.writeBoot(deps.userDataDir, { ...stored, paintFailCount: stored.paintFailCount + 1 });

  return { tier, source, stored };
}

/**
 * §11.2a: what the first frame needs from `boot.json` reaches the window with no round trip, on
 * its argv — the tier, where it came from, which launch forced it off, and the override that
 * clamps it. The preload's parsers in `shared/effectsTier.ts` read back exactly these.
 */
export function bootArguments(boot: BootstrapResult): string[] {
  return [
    `${EFFECTS_TIER_FLAG}${boot.tier}`,
    `${EFFECTS_TIER_SOURCE_FLAG}${boot.source}`,
    ...(boot.stored.paintFailForcedAt === null
      ? []
      : [`${PAINT_FAIL_FORCED_AT_FLAG}${String(boot.stored.paintFailForcedAt)}`]),
    ...(boot.stored.reducedMotionOverride ? [REDUCED_MOTION_OVERRIDE_FLAG] : []),
  ];
}

export function clearPaintFailure(deps: {
  readonly userDataDir: string;
  readonly readBoot: (userDataDir: string) => BootFile;
  readonly writeBoot: (userDataDir: string, file: BootFile) => void;
}): void {
  const stored = deps.readBoot(deps.userDataDir);
  deps.writeBoot(deps.userDataDir, { ...stored, paintFailCount: 0 });
}
