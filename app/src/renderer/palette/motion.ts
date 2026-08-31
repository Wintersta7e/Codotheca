import type { EffectsTier } from '../../shared/effectsTier.js';

/** §8.6: rows enter staggered 14 ms apart. */
export const PALETTE_ROW_STAGGER_MS = 14;

/**
 * §11.6: `reduced` is opacity and colour only with no travelling highlight, and `off` animates
 * nothing — so both flatten the ladder to zero and the CSS swaps the keyframe. `auto` is a
 * resolver and is resolved before it arrives; it is treated as `full` defensively.
 */
export function paletteRowDelayMs(index: number, tier: EffectsTier): number {
  if (tier === 'reduced' || tier === 'off') return 0;
  return Math.max(0, index) * PALETTE_ROW_STAGGER_MS;
}
