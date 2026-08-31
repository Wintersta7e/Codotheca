import { describe, expect, it } from 'vitest';
import { jewelInkFor } from '../theme/contrast';
import {
  GREEBLING_FAMILIES,
  JEWEL_HUES,
  appearanceFor,
  fadeFor,
  hashSeed,
  jewelAlpha,
  languageCode,
} from './appearance';

const seed = { seedBasename: 'aurora', rerollOffset: 0 };
const plain = appearanceFor(seed, 0, 'Rust');

describe('the hash runs over UTF-16 code units of the basename', () => {
  it('reproduces §7.3a on a known seed', () => {
    expect(hashSeed(seed)).toBe(2_888_586_080);
    expect(hashSeed({ seedBasename: 'zz', rerollOffset: 0 })).toBe(3904);
  });

  it('appends the reroll offset only when it is non-zero', () => {
    expect(hashSeed({ seedBasename: 'aurora', rerollOffset: 1 })).toBe(1_382_350_798);
    expect(hashSeed({ seedBasename: 'aurora', rerollOffset: 0 })).toBe(
      hashSeed({ seedBasename: 'aurora', rerollOffset: 0 }),
    );
    expect(hashSeed({ seedBasename: 'aurora#0', rerollOffset: 0 })).not.toBe(
      hashSeed({ seedBasename: 'aurora', rerollOffset: 0 }),
    );
  });

  it('stays inside 32 unsigned bits', () => {
    for (const basename of ['a', 'aurora', 'a-rather-long-directory-basename-here']) {
      const h = hashSeed({ seedBasename: basename, rerollOffset: 0 });
      expect(Number.isInteger(h)).toBe(true);
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThan(2 ** 32);
    }
  });
});

describe('the jewel and its ink', () => {
  it('lands on a bin with at most three degrees of jitter', () => {
    for (const basename of ['a', 'b', 'aurora', 'zz', 'one-two-three']) {
      const a = appearanceFor({ seedBasename: basename, rerollOffset: 0 }, 0, null);
      const nearest = Math.min(...JEWEL_HUES.map((bin) => Math.abs(bin - a.hue)));
      expect(nearest).toBeLessThanOrEqual(3);
    }
    expect(JEWEL_HUES).toEqual([26, 58, 96, 148, 188, 232, 284, 328]);
  });

  it('emits the derived jewel verbatim', () => {
    expect(plain.hue).toBe(27);
    expect(plain.jewel).toBe('oklch(0.53 0.135 27)');
  });

  it('takes its ink from plan 12, fade-independent', () => {
    expect(plain.jewelInk).toBe(jewelInkFor(27));
    expect(plain.jewelInk).toBe('oklch(0.87 0.07 27)');
    const faded = appearanceFor(seed, 0.25, 'Rust');
    expect(faded.jewelInk).toBe(plain.jewelInk);
    expect(faded.jewel).toBe('oklch(0.5 0.1181 27)');
  });

  it('carries alpha into the same colour space', () => {
    expect(jewelAlpha(plain, 0.85)).toBe('oklch(0.53 0.135 27 / .85)');
    expect(jewelAlpha(plain, 0.3)).toBe('oklch(0.53 0.135 27 / .3)');
  });
});

describe('the plate is two gradients and one hue', () => {
  it('emits the derived stops verbatim', () => {
    expect(plain.plate).toBe(
      'linear-gradient(118deg, oklch(0.207 0.009 79) 0 44%, oklch(0.167 0.0095 79) 44% 100%), ' +
        'linear-gradient(157deg, oklch(0.207 0.009 79), oklch(0.096 0.0072 79))',
    );
  });

  it('never lightens past the stop §5.4a measures the dot against', () => {
    // hi = 0.185 + 4 × 0.011 at c = 0.005 + 2 × 0.002, and only because fade is pinned to 0.
    for (const basename of ['a', 'b', 'c', 'aurora', 'zz', 'one', 'two', 'three']) {
      const a = appearanceFor({ seedBasename: basename, rerollOffset: 0 }, 0, null);
      const stops = [...a.plate.matchAll(/oklch\((\d*\.?\d+) /g)].map((m) => Number(m[1]));
      for (const l of stops) expect(l).toBeLessThanOrEqual(0.229);
    }
  });
});

describe('the two families are two draws off one hash', () => {
  it('selects greebling on (h >>> 3) % 4 and livery on h % 4', () => {
    expect(plain.panelFamily).toBe(0);
    expect(plain.liveryFamily).toBe(0);
    const rerolled = appearanceFor({ seedBasename: 'aurora', rerollOffset: 1 }, 0, null);
    expect(rerolled.panelFamily).toBe(1);
    expect(rerolled.liveryFamily).toBe(2);
  });

  it('emits the family the index names', () => {
    expect(plain.greebling).toBe(GREEBLING_FAMILIES[0]);
    expect(GREEBLING_FAMILIES).toHaveLength(4);
  });
});

describe('the designation', () => {
  it('takes the language prefix, the number and the mark', () => {
    expect(plain.designation).toBe('RS-52 / MK-I');
  });

  it('falls back to GN for a language outside the ten keys and for NULL', () => {
    expect(appearanceFor(seed, 0, 'Nim').designation).toBe('GN-52 / MK-I');
    expect(appearanceFor(seed, 0, null).designation).toBe('GN-52 / MK-I');
    expect(languageCode('TypeScript')).toBe('TS');
    expect(languageCode('C++')).toBe('CP');
  });
});

describe('fade is pinned, and it is two flat cases and a zero', () => {
  it('is 0.25 for reference or archived and 0 otherwise', () => {
    expect(fadeFor({ isReference: true, isArchived: false })).toBe(0.25);
    expect(fadeFor({ isReference: false, isArchived: true })).toBe(0.25);
    expect(fadeFor({ isReference: true, isArchived: true })).toBe(0.25);
    expect(fadeFor({ isReference: false, isArchived: false })).toBe(0);
  });

  it('takes no clock, at paint time or anywhere else', () => {
    // Arity alone does NOT catch this and the plan's own mutation proved it: a defaulted
    // `now = Date.now()` parameter leaves `Function.length` at 1, because `length` counts only
    // the parameters before the first default — which is exactly the shape a clock would creep
    // back in as. Read the function's own body instead, the same source guard the core needed
    // for a read-only guarantee no behavioural test could catch.
    expect(fadeFor.length).toBe(1);
    expect(fadeFor.toString()).not.toMatch(/Date|now|performance/i);
    const before = appearanceFor(seed, fadeFor({ isReference: false, isArchived: false }), 'Rust');
    const after = appearanceFor(seed, fadeFor({ isReference: false, isArchived: false }), 'Rust');
    expect(after).toEqual(before);
  });

  it('records the fade it was built at, so no layer re-derives one', () => {
    expect(plain.fade).toBe(0);
    expect(appearanceFor(seed, 0.25, 'Rust').fade).toBe(0.25);
  });
});
