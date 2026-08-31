import { describe, expect, it } from 'vitest';
import { formatPlaytime } from './playtime.js';

describe('playtime', () => {
  it('renders a measured zero as 0h — the one true zero in the product', () => {
    // §8.4.1: "A fact whose job has not run renders `—`, never `0`. `PLAYTIME 0h` is the one
    // exception and is true: that ledger starts at install." `10-first-run.md:326` says the same.
    // This is the assertion that keeps `0h` from drifting back to `0m`.
    expect(formatPlaytime(0)).toBe('0h');
  });

  it('renders the unit the spec renders, at every magnitude', () => {
    expect(formatPlaytime(6 * 3600)).toBe('6h');
    expect(formatPlaytime(40 * 3600)).toBe('40h');
    for (const seconds of [0, 59, 45 * 60, 6 * 3600, 40 * 3600]) {
      expect(formatPlaytime(seconds)).toMatch(/^\d+(\.\d)?h$/);
    }
  });

  it('keeps a sub-hour session visible instead of flooring it into the install zero', () => {
    // Unresolved in the spec, decided here: one decimal of *precision*, the rounding already
    // settled for gigabytes. Flooring would print `0h` for 45 real minutes and collapse
    // "never played" together with "played most of an hour" — and `0h`'s whole point is that it
    // is a fact, not a rounding.
    expect(formatPlaytime(45 * 60)).toBe('0.8h');
    expect(formatPlaytime(12 * 60)).toBe('0.2h');
    expect(formatPlaytime(2 * 3600 + 7 * 60)).toBe('2.1h');
  });

  it('is precision, not a forced decimal place — a whole hour carries no .0', () => {
    expect(formatPlaytime(6 * 3600)).not.toContain('.');
    expect(formatPlaytime(0)).not.toContain('.');
  });

  it('is not the bench grammar, which is the other ledger', () => {
    // §7.8's scrim reads `AT THE BENCH · 2h 07m` over elapsed wall time on an open session and is
    // 12c's `useBenchElapsed`. Two ledgers, never merged — so two grammars, visibly different.
    expect(formatPlaytime(2 * 3600 + 7 * 60)).not.toBe('2h 07m');
    expect(formatPlaytime(2 * 3600 + 7 * 60)).not.toMatch(/m$/);
  });

  it('floors a negative duration rather than printing one', () => {
    // Clock skew across a network share is ordinary; `-2h` played is not a fact about anything.
    expect(formatPlaytime(-90)).toBe('0h');
  });
});
