import { describe, expect, it } from 'vitest';
import type { ConditionSignal } from '../../../generated/protocol';
import { LADDER_POSITION, ROSE_SECTOR_DEG, divergence, needleAngle } from './dial';

const LADDER: readonly ConditionSignal[] = ['live', 'idle', 'dormant', 'neglected', 'abandoned'];
const OFF_LADDER: readonly ConditionSignal[] = ['offline', 'empty'];

describe('the two-clock dial', () => {
  it('puts the five ladder rungs in §5.4s order and neither off-ladder value on it', () => {
    expect(LADDER.map((s) => LADDER_POSITION[s])).toEqual([0, 1, 2, 3, 4]);
    for (const signal of OFF_LADDER) expect(LADDER_POSITION[signal]).toBeNull();
  });

  it('takes the centre of each of five equal sectors over a full circle', () => {
    expect(ROSE_SECTOR_DEG).toBe(72);
    expect(LADDER.map(needleAngle)).toEqual([36, 108, 180, 252, 324]);
  });

  it('never lets an off-ladder needle coincide with a rung', () => {
    // An off-ladder value takes 0°, which is a sector BOUNDARY and therefore no rung's centre.
    // That is what stops the dial claiming a reading it does not have.
    const rungs = new Set(LADDER.map(needleAngle));
    for (const signal of OFF_LADDER) {
      expect(needleAngle(signal)).toBe(0);
      expect(rungs.has(needleAngle(signal))).toBe(false);
    }
  });

  it('reads IN STEP when the two clocks sit on the same rung', () => {
    for (const signal of LADDER) expect(divergence(signal, signal)).toBe('IN STEP');
  });

  it('reads LIT BUT DUSTY when the interaction clock is the warmer of the two', () => {
    expect(divergence('live', 'abandoned')).toBe('LIT BUT DUSTY');
    expect(divergence('idle', 'dormant')).toBe('LIT BUT DUSTY');
    expect(divergence('neglected', 'abandoned')).toBe('LIT BUT DUSTY');
  });

  it('reads CLEAN BUT DARK when the commit clock is the warmer of the two', () => {
    expect(divergence('abandoned', 'live')).toBe('CLEAN BUT DARK');
    expect(divergence('dormant', 'idle')).toBe('CLEAN BUT DARK');
    expect(divergence('abandoned', 'neglected')).toBe('CLEAN BUT DARK');
  });

  it('covers every ordered pair of ladder values with one of the three readings', () => {
    let pairs = 0;
    for (const signal of LADDER) {
      for (const material of LADDER) {
        expect(['IN STEP', 'LIT BUT DUSTY', 'CLEAN BUT DARK']).toContain(
          divergence(signal, material),
        );
        pairs += 1;
      }
    }
    expect(pairs).toBe(25);
  });

  it('draws no divergence line when either value is off the ladder', () => {
    for (const off of OFF_LADDER) {
      for (const on of LADDER) {
        expect(divergence(off, on)).toBeNull();
        expect(divergence(on, off)).toBeNull();
      }
      expect(divergence(off, off)).toBeNull();
    }
  });

  it('draws no divergence line when the material clock was never computed', () => {
    // §33.7: `condition_material IS NULL` draws no inner needle and no divergence line — never
    // a needle at zero. That is the unknown-as-zero invariant at the one site phase 3 makes
    // expressible.
    for (const signal of [...LADDER, ...OFF_LADDER]) {
      expect(divergence(signal, null)).toBeNull();
    }
  });

  it('carries no percentage window, because §33.7 compares LADDER POSITIONS', () => {
    // The prototype read `Math.abs(glowPct - matPct) < 12 ? 'IN STEP' : …`. A 12-point window
    // over a percentage is a sixth threshold nothing else in the product carries, and it would
    // call two adjacent rungs the same reading.
    expect(divergence('live', 'idle')).toBe('LIT BUT DUSTY');
    expect(divergence('idle', 'live')).toBe('CLEAN BUT DARK');
  });
});
