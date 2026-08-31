import { describe, expect, it } from 'vitest';
import { formatBenchElapsed } from '../card/useBenchElapsed';
import { formatPlaytime } from './playtime';

describe('playtime', () => {
  it('renders an empty ledger as 0h, the one zero this product is allowed to print', () => {
    // §8.1: a fact whose job has not run renders an em dash and never a zero; `PLAYTIME 0h` is
    // the one exception, and it is true because that ledger starts at install.
    expect(formatPlaytime(0)).toBe('0h');
    expect(formatPlaytime(59)).toBe('0h');
  });

  it('floors a negative rather than printing one — a clock skew is not a negative ledger', () => {
    expect(formatPlaytime(-90)).toBe('0h');
  });

  it('counts launched-session time only, in the duration grammar above zero', () => {
    expect(formatPlaytime(12 * 60)).toBe('12m');
    expect(formatPlaytime(2 * 3600 + 7 * 60)).toBe('2h 07m');
    expect(formatPlaytime(40 * 3600)).toBe('40h 00m');
  });

  it('is the only playtime grammar in the renderer — Peek and the rail call this one', () => {
    expect(formatPlaytime(2 * 3600 + 7 * 60)).toMatch(/^\d+h \d{2}m$/);
    expect(formatPlaytime(12 * 60)).toMatch(/^\d+m$/);
  });
});

/**
 * The two ledgers are never summed and share no code, so this is the test that keeps them
 * spelling one duration the same way. It reads the other side rather than restating it — and it
 * pins the one place they are required to differ, which is the whole point of two ledgers.
 */
describe('playtime and the bench figure', () => {
  it('agree above zero, at every boundary the grammar turns on', () => {
    for (const seconds of [60, 599, 3599, 3600, 3660, 2 * 3600 + 7 * 60, 40 * 3600]) {
      expect(formatPlaytime(seconds), `${String(seconds)}s`).toBe(formatBenchElapsed(seconds));
    }
  });

  it('diverge at zero, because only one of them has a ledger to declare complete', () => {
    // The bench figure is elapsed wall time on a session that has just opened, so its zero is a
    // stopwatch reading. Playtime's zero is a statement that nothing has been recorded yet.
    expect(formatBenchElapsed(0)).toBe('0m');
    expect(formatPlaytime(0)).toBe('0h');
  });
});
