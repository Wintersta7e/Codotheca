/**
 * §11.3a's two motion rows, as §11.6's resolver reads them.
 *
 * The window's argv carries the tier and the override the shell read from `boot.json` at launch
 * (§11.2a), which is all there is before the core joins. After that the database is
 * authoritative: `settings.get` replaces the launch values, and every answer the drawer receives
 * from `settings.set` replaces them again — the write raises no event, so the drawer hands its
 * answer here rather than leaving the window on a tier the user has already changed.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

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
  const { request, onCoreStatus, effectsTier, effectsTierSource, reducedMotionOverride } = deps;
  const live = useRef(true);
  // Counts the drawer's answers, so a read issued before one cannot land after it and undo it.
  const writes = useRef(0);

  const read = useCallback(() => {
    const issuedAfter = writes.current;
    void request('settings.get', {}).then(
      (answer) => {
        if (live.current && writes.current === issuedAfter) setSettings(answer);
      },
      () => {
        // Unread keeps the last answer, or the launch values — what the window already shows.
      },
    );
  }, [request]);

  useEffect(() => {
    live.current = true;
    read();
    return () => {
      live.current = false;
    };
  }, [read]);

  // A core that restarts rejects every pending request, this read among them, and the lane that
  // comes back may hold a value this window never heard. Ready is when it can be read again.
  useEffect(
    () =>
      onCoreStatus((status) => {
        if (status.kind === 'ready') read();
      }),
    [onCoreStatus, read],
  );

  const accept = useCallback((answer: Settings) => {
    writes.current += 1;
    setSettings(answer);
  }, []);

  return useMemo(
    () => ({
      effectsTier: storedTierFor(effectsTier, effectsTierSource, settings?.effectsTier ?? null),
      reducedMotionOverride: settings?.reducedMotionOverride ?? reducedMotionOverride,
      accept,
    }),
    [effectsTier, effectsTierSource, reducedMotionOverride, settings, accept],
  );
}
