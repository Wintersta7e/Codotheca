import { describe, expect, it } from 'vitest';
import {
  EFFECTS_TIER_FLAG,
  EFFECTS_TIERS,
  effectsTierFromArgv,
  parseEffectsTier,
} from './effectsTier';

describe('parseEffectsTier', () => {
  it('accepts the four tiers §11.3 names and nothing else', () => {
    expect(EFFECTS_TIERS).toEqual(['auto', 'full', 'reduced', 'off']);
    for (const tier of EFFECTS_TIERS) {
      expect(parseEffectsTier(tier)).toBe(tier);
    }
    expect(parseEffectsTier('FULL')).toBe('full');
    expect(parseEffectsTier(' off ')).toBe('off');
  });

  it('returns null rather than a default for anything unrecognised', () => {
    // A caller must be able to tell "not set" from "set to auto", because that difference
    // is the whole override chain.
    expect(parseEffectsTier(undefined)).toBeNull();
    expect(parseEffectsTier('')).toBeNull();
    expect(parseEffectsTier('fancy')).toBeNull();
  });
});

describe('effectsTierFromArgv', () => {
  it('reads the last matching flag', () => {
    expect(effectsTierFromArgv(['app', '--effects-tier=reduced'])).toBe('reduced');
    expect(effectsTierFromArgv(['app', '--effects-tier=full', '--effects-tier=off'])).toBe('off');
  });

  it('ignores an unparseable value and an absent flag', () => {
    expect(effectsTierFromArgv(['app', '--effects-tier=sparkly'])).toBeNull();
    expect(effectsTierFromArgv(['app'])).toBeNull();
  });
});

describe('the renderer handover', () => {
  it('round-trips through the argument the shell passes to the renderer', () => {
    // additionalArguments is the only handover with no round trip: the preload reads argv
    // synchronously, so the tier is on the document before React mounts.
    for (const tier of EFFECTS_TIERS) {
      expect(effectsTierFromArgv(['renderer', `${EFFECTS_TIER_FLAG}${tier}`])).toBe(tier);
    }
  });
});
