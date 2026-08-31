import { describe, expect, it } from 'vitest';
import { formatBenchElapsed } from '../card/useBenchElapsed';
import { formatPlaytime } from './playtime';

describe('playtime', () => {
  it('counts launched-session time only, and a measured zero is a zero', () => {
    expect(formatPlaytime(0)).toBe('0m');
    expect(formatPlaytime(59)).toBe('0m');
    expect(formatPlaytime(12 * 60)).toBe('12m');
    expect(formatPlaytime(2 * 3600 + 7 * 60)).toBe('2h 07m');
    expect(formatPlaytime(40 * 3600)).toBe('40h 00m');
  });

  it('floors a negative rather than printing one — a clock skew is not a negative ledger', () => {
    expect(formatPlaytime(-90)).toBe('0m');
  });

  it('is the only playtime grammar in the renderer — Peek and the rail call this one', () => {
    expect(formatPlaytime(2 * 3600 + 7 * 60)).toMatch(/^\d+h \d{2}m$/);
    expect(formatPlaytime(12 * 60)).toMatch(/^\d+m$/);
  });
});

/**
 * The two ledgers are never summed and share no code, so this is the test that keeps them
 * spelling one duration the same way. It reads the other side rather than restating it.
 */
describe('the two ledgers print one duration identically', () => {
  it('agrees with the bench figure across the boundaries the grammar turns on', () => {
    for (const seconds of [0, 59, 60, 599, 3599, 3600, 3660, 2 * 3600 + 7 * 60, 40 * 3600]) {
      expect(formatPlaytime(seconds), `${String(seconds)}s`).toBe(formatBenchElapsed(seconds));
    }
  });
});
