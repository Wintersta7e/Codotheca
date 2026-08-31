import { describe, expect, it } from 'vitest';
import {
  DECISION_FLOOR,
  LIGHTEST_PLATE_STOP,
  NON_TEXT_FLOOR,
  compositeOver,
  jewelInkFor,
  meetsDecisionFloor,
  parseHex,
  ratioOf,
} from './contrast';
import { tokenValue } from './tokens';

const S0 = tokenValue('surface-0');
const S1 = tokenValue('surface-1');
const S2 = tokenValue('surface-2');
const S3 = tokenValue('surface-3');
const S4 = tokenValue('surface-4');
const PLATE = `oklch(${String(LIGHTEST_PLATE_STOP.l)} ${String(LIGHTEST_PLATE_STOP.c)} ${String(LIGHTEST_PLATE_STOP.hue)})`;

const near = (actual: number, expected: number): void => {
  expect(Math.round(actual * 100) / 100).toBe(expected);
};

describe('the recomputed type ladder', () => {
  it('reproduces §8.7 on --surface-1', () => {
    near(ratioOf(tokenValue('text-0'), S1), 17.43);
    near(ratioOf(tokenValue('text-1'), S1), 14.74);
    near(ratioOf(tokenValue('text-2'), S1), 10.43);
    near(ratioOf(tokenValue('text-3'), S1), 6.41);
    near(ratioOf(tokenValue('text-4'), S1), 4.24);
    near(ratioOf(tokenValue('text-5'), S1), 2.51);
  });

  it('reproduces §8.7 on --surface-4', () => {
    near(ratioOf(tokenValue('text-0'), S4), 15.51);
    near(ratioOf(tokenValue('text-1'), S4), 13.12);
    near(ratioOf(tokenValue('text-2'), S4), 9.28);
    near(ratioOf(tokenValue('text-3'), S4), 5.7);
    near(ratioOf(tokenValue('text-4'), S4), 3.77);
    near(ratioOf(tokenValue('text-5'), S4), 2.23);
  });

  it('the design published column is dead — 3.8 and 2.2 are not the numbers', () => {
    expect(Math.round(ratioOf(tokenValue('text-4'), S1) * 10) / 10).not.toBe(3.8);
    expect(Math.round(ratioOf(tokenValue('text-5'), S1) * 10) / 10).not.toBe(2.2);
  });
});

describe('the floor is a number, not a token', () => {
  it('is 5.9:1 and --text-3 clears it on 0, 1, 2 and 3 but not on 4', () => {
    expect(DECISION_FLOOR).toBe(5.9);
    near(ratioOf(tokenValue('text-3'), S0), 6.54);
    near(ratioOf(tokenValue('text-3'), S2), 6.21);
    near(ratioOf(tokenValue('text-3'), S3), 6.1);
    expect(meetsDecisionFloor(tokenValue('text-3'), S1)).toBe(true);
    expect(meetsDecisionFloor(tokenValue('text-3'), S4)).toBe(false);
    // …so a row that can be hovered onto --surface-4 sets its decision text in --text-2.
    expect(meetsDecisionFloor(tokenValue('text-2'), S4)).toBe(true);
  });
});

describe('the accent', () => {
  it('reproduces every measured ratio §8.7 prices the deferral on', () => {
    near(ratioOf(tokenValue('sig'), S1), 6.87);
    near(ratioOf(tokenValue('sig'), S4), 6.11);
    near(ratioOf(tokenValue('sig'), tokenValue('pill-bg')), 5.22);
    near(ratioOf(tokenValue('sig-ink'), tokenValue('sig')), 6.73);
    near(ratioOf(tokenValue('sig'), PLATE), 6.1);
    near(ratioOf(tokenValue('sig-hover'), S1), 10.28);
  });
});

describe('unknown has its own pair, and --absent is meant to read as nothing', () => {
  it('measures both', () => {
    near(ratioOf(tokenValue('unknown-ink'), tokenValue('unknown')), 9.45);
    near(ratioOf(tokenValue('absent'), S1), 1.54);
    expect(NON_TEXT_FLOOR).toBe(3);
  });
});

describe('jewelInk is what the selection and focus ring is drawn in', () => {
  it('clears the decision floor against the lightest plate stop for every jewel bin', () => {
    // §7.8 publishes 11.2–11.7:1; recomputing lands inside 11.1–11.8 on the same definition.
    const bins = [26, 58, 96, 148, 188, 232, 284, 328];
    for (const bin of bins) {
      for (let jitter = -3; jitter <= 3; jitter += 1) {
        const ratio = ratioOf(jewelInkFor(bin + jitter), PLATE);
        expect(ratio).toBeGreaterThan(11.1);
        expect(ratio).toBeLessThan(11.8);
      }
    }
  });
});

describe('the pin ink is deliberately not the jewel', () => {
  it('clears the text floor over the etched ground on both plates', () => {
    const etched = parseHex('#080c0f');
    const overDarkest = compositeOver(etched, 0.58, parseHex(S1));
    const overLightest = compositeOver(etched, 0.58, parseHex('#1f1c18'));
    expect(ratioOf(tokenValue('text-2'), overDarkest)).toBeGreaterThan(10);
    expect(ratioOf(tokenValue('text-2'), overLightest)).toBeGreaterThan(10);
  });
});
