import { describe, expect, it } from 'vitest';
import { BYTES_PER_GB, BYTES_PER_MB, GB_FLOOR_BYTES, formatTrackedBytes } from './size';

describe('§8.1: one decimal of GB at or above 0.1 GB, whole MB below', () => {
  it('formats the four figures the shelf and the project page both print', () => {
    expect(formatTrackedBytes(41 * BYTES_PER_GB)).toBe('41 GB');
    expect(formatTrackedBytes(Math.round(0.15 * BYTES_PER_GB))).toBe('0.2 GB');
    expect(formatTrackedBytes(100 * BYTES_PER_MB)).toBe('100 MB');
    expect(formatTrackedBytes(0)).toBe('0 MB');
  });

  // "One decimal" is precision, not padding: the era header prints `41 GB tracked`, and a
  // `toFixed(1)` reading — which prints `41.0 GB` — is the one under which §8.1's rule sentence
  // and its own examples contradict each other. This is the single byte formatter in the product
  // (R12), so the padded form is pinned out rather than left to the next reader to rediscover.
  it('drops a trailing zero and keeps a real tenth', () => {
    expect(formatTrackedBytes(41 * BYTES_PER_GB)).not.toContain('.');
    expect(formatTrackedBytes(2 * BYTES_PER_GB)).toBe('2 GB');
    expect(formatTrackedBytes(Math.round(1.24 * BYTES_PER_GB))).toBe('1.2 GB');
    expect(formatTrackedBytes(Math.round(1.26 * BYTES_PER_GB))).toBe('1.3 GB');
    // A tenth that is exactly half a rung still prints, so "drops a trailing zero" is not
    // mistaken for "drops the decimal".
    expect(formatTrackedBytes(Math.round(2.5 * BYTES_PER_GB))).toBe('2.5 GB');
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
  // decimal at every rung. The MB branch carries none at all: it is whole megabytes.
  it('never carries a decimal at MB, which is where the two disagreed', () => {
    expect(formatTrackedBytes(8 * BYTES_PER_MB)).toBe('8 MB');
    for (const bytes of [0, 1, 940, 8_400_000, GB_FLOOR_BYTES - 1]) {
      expect(formatTrackedBytes(bytes)).not.toContain('.');
    }
  });
});

describe('the label is the caller‘s', () => {
  it('returns the figure alone, because §8.1 appends "tracked" and §8.4.1 appends nothing', () => {
    expect(formatTrackedBytes(3 * BYTES_PER_GB)).not.toContain('tracked');
  });
});
