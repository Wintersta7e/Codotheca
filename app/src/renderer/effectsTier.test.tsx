// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { EFFECTS_TIER_ATTRIBUTE, applyEffectsTier } from './effectsTier';

describe('applyEffectsTier', () => {
  it('writes the tier onto the element it is given', () => {
    const root = document.createElement('html');
    applyEffectsTier(root, 'off');
    expect(root.getAttribute(EFFECTS_TIER_ATTRIBUTE)).toBe('off');
  });

  it('overwrites a tier already there', () => {
    // Plan 12's auto resolver calls this again with the resolved value, so the second write
    // must replace the first rather than accumulate.
    const root = document.createElement('html');
    applyEffectsTier(root, 'auto');
    applyEffectsTier(root, 'reduced');
    expect(root.getAttribute(EFFECTS_TIER_ATTRIBUTE)).toBe('reduced');
  });
});
