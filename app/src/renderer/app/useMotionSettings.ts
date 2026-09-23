/**
 * §11.3a's two motion rows, as §11.6's resolver reads them.
 *
 * The window's argv carries the tier the shell resolved at launch (§11.2a), which is all there is
 * before the core joins. After that the database is authoritative: `settings.get` replaces the
 * launch value, and every answer the drawer receives from `settings.set` replaces it again — the
 * write raises no event, so the drawer hands its answer here rather than leaving the window on a
 * tier the user has already changed.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import type { Settings } from '../../generated/protocol.js';
import type { EffectsTier, EffectsTierSource } from '../../shared/effectsTier.js';
import type { AppDeps } from './deps.js';

export interface MotionSettings {
  readonly effectsTier: EffectsTier;
  readonly reducedMotionOverride: boolean;
  /** Every `Settings` the core answered with. */
  readonly accept: (settings: Settings) => void;
}

/**
 * §11.2a's order: argv beats the environment, both beat `boot.json`, and a paint-failure forcing
 * sits between them. `boot.json` is only a mirror of the stored setting, so the stored setting
 * replaces a `boot-file` tier and nothing else — an operator's override holds for the launch.
 */
export function storedTierFor(
  launch: EffectsTier,
  source: EffectsTierSource,
  stored: EffectsTier | null,
): EffectsTier {
  return source === 'boot-file' && stored !== null ? stored : launch;
}

export function useMotionSettings(deps: AppDeps): MotionSettings {
  const [settings, setSettings] = useState<Settings | null>(null);
  const { request, effectsTier, effectsTierSource } = deps;

  useEffect(() => {
    let live = true;
    void request('settings.get', {}).then(
      (answer) => {
        // A drawer write answered first is newer than this read, so it is not overwritten.
        if (live) setSettings((current) => current ?? answer);
      },
      () => {
        // Unread keeps the launch tier, which is what the window has painted with all along.
      },
    );
    return () => {
      live = false;
    };
  }, [request]);

  const accept = useCallback((answer: Settings) => {
    setSettings(answer);
  }, []);

  return useMemo(
    () => ({
      effectsTier: storedTierFor(effectsTier, effectsTierSource, settings?.effectsTier ?? null),
      reducedMotionOverride: settings?.reducedMotionOverride ?? false,
      accept,
    }),
    [effectsTier, effectsTierSource, settings, accept],
  );
}
