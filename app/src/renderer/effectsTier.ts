import type { EffectsTier } from '../shared/effectsTier';

/** Every motion rule (§11.6) hangs off this attribute. Plan 12 writes the CSS that reads it. */
export const EFFECTS_TIER_ATTRIBUTE = 'data-effects-tier';

/**
 * A pure setter. It resolves nothing: `auto` is written as `auto`, and §11.6's resolver calls
 * this again with the resolved tier once it has queried the compositor.
 */
export function applyEffectsTier(root: HTMLElement, tier: EffectsTier): void {
  root.setAttribute(EFFECTS_TIER_ATTRIBUTE, tier);
}
