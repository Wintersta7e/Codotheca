import { useEffect, useState } from 'react';
import type { EffectsTier } from '../../shared/effectsTier';

/**
 * §11.6's tier contract. `auto` is a resolver, not a stored state; every rule reads the
 * *resolved* tier and an explicit setting outranks it.
 *
 * Every animated state has a still rendering that preserves its meaning — condition, tier, rank
 * and safety are all readable at `off`. The tier gates transitions and travelling highlights;
 * it never gates a state. A hovered card takes the jewel frame and the bloom at `off` too,
 * instantly. Hazard tape, the pin mark, the unknown gap and the focus ring are identical at all
 * three tiers.
 */
export type ResolvedTier = 'full' | 'reduced' | 'off';

export interface MotionSignals {
  readonly prefersReducedMotion: boolean;
  /**
   * Phase 1 creates no WebGL context and no canvas (criterion 67), so the renderer cannot
   * observe this for itself; the shell reports it. False on every path phase 1 exercises.
   */
  readonly softwareCompositing: boolean;
  readonly contextLostThisSession: boolean;
}

export const REDUCED_CLAMP_MS = 160;

export function resolveEffectsTier(stored: EffectsTier, signals: MotionSignals): ResolvedTier {
  if (stored !== 'auto') return stored;
  if (signals.softwareCompositing || signals.contextLostThisSession) return 'off';
  if (signals.prefersReducedMotion) return 'reduced';
  return 'full';
}

/** §11.3a's override: it clamps the tier to at most `reduced`, and says so. */
export function clampToOverride(tier: ResolvedTier, reducedMotionOverride: boolean): ResolvedTier {
  if (!reducedMotionOverride) return tier;
  return tier === 'off' ? 'off' : 'reduced';
}

export function allowsTransitions(tier: ResolvedTier): boolean {
  return tier !== 'off';
}

export function allowsTransforms(tier: ResolvedTier): boolean {
  return tier === 'full';
}

/** LED run, specular sweep, brackets, scan line — the travelling highlights §7.8 lists. */
export function allowsTravellingHighlights(tier: ResolvedTier): boolean {
  return tier === 'full';
}

/** §11.6's scheduled flicker, the one permitted scheduled frame. */
export function allowsScheduledFrames(tier: ResolvedTier): boolean {
  return tier === 'full';
}

const REDUCE_QUERY = '(prefers-reduced-motion: reduce)';

export function useResolvedTier(
  stored: EffectsTier,
  softwareCompositing: boolean,
  reducedMotionOverride: boolean,
): ResolvedTier {
  const [prefersReducedMotion, setPrefers] = useState(
    () => globalThis.matchMedia?.(REDUCE_QUERY).matches ?? false,
  );
  useEffect(() => {
    const query = globalThis.matchMedia?.(REDUCE_QUERY);
    if (query === undefined) return undefined;
    const onChange = (): void => {
      setPrefers(query.matches);
    };
    query.addEventListener('change', onChange);
    setPrefers(query.matches);
    return () => {
      query.removeEventListener('change', onChange);
    };
  }, []);
  return clampToOverride(
    resolveEffectsTier(stored, {
      prefersReducedMotion,
      softwareCompositing,
      contextLostThisSession: false,
    }),
    reducedMotionOverride,
  );
}
