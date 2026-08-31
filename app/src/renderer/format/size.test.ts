import { describe, expect, it } from 'vitest';
import { BYTES_PER_GB, BYTES_PER_MB, GB_FLOOR_BYTES, formatTrackedBytes } from './size';

describe('§8.1: one decimal of GB at or above 0.1 GB, whole MB below', () => {
  it('formats the four figures the shelf and the project page both print', () => {
    // `41 GB`, not `41.0 GB`. "One decimal of GB" is precision, and the prototype
    // (`Codotheca v7 Shelf.dc.html:2683`) rounds to one place and concatenates, which drops a
    // trailing zero. §8.1's own era-header examples print `41 GB tracked`.
    expect(formatTrackedBytes(41 * BYTES_PER_GB)).toBe('41 GB');
    expect(formatTrackedBytes(Math.round(0.15 * BYTES_PER_GB))).toBe('0.2 GB');
    expect(formatTrackedBytes(100 * BYTES_PER_MB)).toBe('100 MB');
    expect(formatTrackedBytes(0)).toBe('0 MB');
  });

  it('switches at exactly 0.1 GB and not a byte earlier', () => {
    expect(formatTrackedBytes(GB_FLOOR_BYTES)).toBe('0.1 GB');
    expect(formatTrackedBytes(GB_FLOOR_BYTES - 1)).toBe('102 MB');
  });

  // 0.1 GB is 102.4 MiB, so `102 MB` is the LARGEST figure the MB branch can ever print. Any
  // three-digit MB figure above it is unreachable, and asserting one asserts a switch point of
  // 1 GB rather than §8.1's 0.1 GB.
  it('has no MB figure above 102, because that is where the floor lands', () => {
    expect(formatTrackedBytes(200 * BYTES_PER_MB)).toBe('0.2 GB');
    expect(formatTrackedBytes(103 * BYTES_PER_MB)).not.toContain('MB');
  });
});

describe('the rungs the spec does not carry are not invented', () => {
  it('has no B rung and no KB rung — "whole MB below" is the whole of the low end', () => {
    expect(formatTrackedBytes(940)).toBe('0 MB');
    expect(formatTrackedBytes(8_400_000)).toBe('8 MB');
    for (const bytes of [0, 1, 940, 8_400_000, 200 * BYTES_PER_MB]) {
      expect(formatTrackedBytes(bytes)).toMatch(/^\d+(\.\d)? (MB|GB)$/);
    }
  });

  it('divides by 1024, not by 1000 — the two disagree by 7% at a gigabyte', () => {
    expect(formatTrackedBytes(1_000_000_000)).toBe('0.9 GB');
  });

  // The deleted implementation printed `8.0 MB` where §8.1 prints `8 MB`, because it carried a
  // decimal at every rung. Neither branch forces one: a whole figure prints whole on both sides,
  // and the decimal appears only when the value has one.
  it('never forces a decimal place on a whole figure, at either rung', () => {
    expect(formatTrackedBytes(8 * BYTES_PER_MB)).toBe('8 MB');
    expect(formatTrackedBytes(2 * BYTES_PER_GB)).toBe('2 GB');
    expect(formatTrackedBytes(Math.round(2.5 * BYTES_PER_GB))).toBe('2.5 GB');
  });
});

describe('the label is the caller‘s', () => {
  it('returns the figure alone, because §8.1 appends "tracked" and §8.4.1 appends nothing', () => {
    expect(formatTrackedBytes(3 * BYTES_PER_GB)).not.toContain('tracked');
  });
});
