import { describe, expect, it } from 'vitest';
import { formatPlaytime } from './playtime.js';

describe('playtime', () => {
  it('counts launched-session time only, and a measured zero is a zero', () => {
    expect(formatPlaytime(0)).toBe('0m');
    expect(formatPlaytime(59)).toBe('0m');
    expect(formatPlaytime(12 * 60)).toBe('12m');
    expect(formatPlaytime(2 * 3600 + 7 * 60)).toBe('2h 07m');
    expect(formatPlaytime(40 * 3600)).toBe('40h 00m');
  });

  it('is the only playtime grammar in the renderer — Peek and the rail call this one', () => {
    expect(formatPlaytime(2 * 3600 + 7 * 60)).toMatch(/^\d+h \d{2}m$/);
    expect(formatPlaytime(12 * 60)).toMatch(/^\d+m$/);
  });

  it('floors a negative duration rather than printing one', () => {
    // Clock skew across a network share is ordinary; `-2m` played is not a fact about anything.
    expect(formatPlaytime(-90)).toBe('0m');
  });
});
