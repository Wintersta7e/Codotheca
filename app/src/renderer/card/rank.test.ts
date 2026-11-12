import { describe, expect, it } from 'vitest';
import type { CheckState, ConditionSignal } from '../../generated/protocol';
import { LIGHTEST_PLATE_STOP, oklchToRgb, ratioOf } from '../theme/contrast';
import { tokenValue } from '../theme/tokens';
import { type RankPaint, type RankRow, rankOf, rankPaint } from './rank';

/** A computed row with every adjustment neutral: `idle`, nothing observed, CI unread. */
const row = (over: Partial<RankRow> = {}): RankRow => ({
  completionLit: 9,
  completionApplicable: 10,
  conditionSignal: 'idle',
  isArchived: false,
  isReference: false,
  isDirty: null,
  ahead: null,
  ciGreen: null,
  ...over,
});

describe('rankOf — the completion thresholds 1 / .85 / .7 / .5 / .3', () => {
  it.each([
    [10, 10, 'S'],
    [99, 100, 'A'],
    [17, 20, 'A'],
    [84, 100, 'B'],
    [7, 10, 'B'],
    [69, 100, 'C'],
    [5, 10, 'C'],
    [49, 100, 'D'],
    [3, 10, 'D'],
    [29, 100, 'E'],
    [0, 10, 'E'],
    // The denominator is what could be scored, never ten: 7 of 7 is 100%.
    [7, 7, 'S'],
  ] as const)('%i of %i reads %s with no adjustment', (lit, evaluable, rank) => {
    expect(rankOf(row({ completionLit: lit, completionApplicable: evaluable }))).toBe(rank);
  });
});

describe('rankOf — the condition band, mapped onto the shipped ladder', () => {
  it.each([
    ['live', 'S'],
    ['idle', 'A'],
    ['dormant', 'B'],
    ['neglected', 'C'],
    ['abandoned', 'C'],
    ['offline', 'A'],
    ['empty', 'A'],
    [null, 'A'],
  ] as const satisfies readonly (readonly [ConditionSignal | null, string])[])(
    '%s moves an A to %s',
    (signal, rank) => {
      expect(rankOf(row({ conditionSignal: signal }))).toBe(rank);
    },
  );

  it('takes no condition adjustment on an archived or reference row', () => {
    expect(rankOf(row({ conditionSignal: 'abandoned', isArchived: true }))).toBe('A');
    expect(rankOf(row({ conditionSignal: 'neglected', isReference: true }))).toBe('A');
    expect(rankOf(row({ conditionSignal: 'live', isArchived: true }))).toBe('A');
  });
});

describe('rankOf — open flags', () => {
  it('demotes one step for unpushed commits, uncommitted changes, or both', () => {
    expect(rankOf(row({ ahead: 2 }))).toBe('B');
    expect(rankOf(row({ isDirty: true }))).toBe('B');
    expect(rankOf(row({ ahead: 2, isDirty: true }))).toBe('B');
  });

  it('demotes one step for a failing stored CI check, and for nothing else it holds', () => {
    expect(rankOf(row({ ciGreen: 'fail' }))).toBe('B');
    for (const state of ['pass', 'na', 'unknown'] as const satisfies readonly CheckState[]) {
      expect(rankOf(row({ ciGreen: state }))).toBe('A');
    }
  });

  it('stacks every demotion and clamps at both ends', () => {
    expect(rankOf(row({ conditionSignal: 'dormant', ahead: 1, ciGreen: 'fail' }))).toBe('D');
    expect(
      rankOf(
        row({ completionLit: 3, conditionSignal: 'abandoned', isDirty: true, ciGreen: 'fail' }),
      ),
    ).toBe('E');
    expect(rankOf(row({ completionLit: 10, conditionSignal: 'live' }))).toBe('S');
  });
});

describe('rankOf — unknown never penalises and never ranks', () => {
  it('reads an unobserved dirty, push or CI state as no demotion', () => {
    const unknown = row({ isDirty: null, ahead: null, ciGreen: null, conditionSignal: null });
    expect(rankOf(unknown)).toBe('A');
    expect(rankOf(row({ isDirty: false, ahead: 0 }))).toBe('A');
  });

  it('gives no rank while completion is uncomputed or nothing could be scored', () => {
    const penalised = { conditionSignal: 'abandoned', isDirty: true, ciGreen: 'fail' } as const;
    expect(rankOf(row({ ...penalised, completionLit: null, completionApplicable: null }))).toBe(
      null,
    );
    expect(rankOf(row({ completionApplicable: null }))).toBe(null);
    expect(rankOf(row({ completionLit: 0, completionApplicable: 0 }))).toBe(null);
    expect(rankPaint(row({ completionLit: null, completionApplicable: null }))).toBe(null);
  });
});

describe('rankPaint — the design table through the --rank-* tokens', () => {
  it('names each rank, its label, its token, and glows only S, A and B', () => {
    const at = (lit: number, signal: ConditionSignal = 'idle'): RankPaint | null =>
      rankPaint(row({ completionLit: lit, completionApplicable: 10, conditionSignal: signal }));
    expect(at(10, 'live')).toEqual({
      rank: 'S',
      label: 'S RANK',
      inkToken: 'rank-s',
      glow: '0 0 26px rgb(232 194 104 / .45)',
      accessibleName: 'Rank S',
    });
    expect(at(9)?.inkToken).toBe('rank-a');
    expect(at(9)?.glow).toBe('0 0 24px rgb(178 138 236 / .40)');
    expect(at(7)?.inkToken).toBe('rank-b');
    expect(at(7)?.glow).toBe('0 0 22px rgb(112 168 246 / .38)');
    expect(at(5)).toMatchObject({ rank: 'C', label: 'C RANK', inkToken: 'rank-c', glow: null });
    expect(at(3)).toMatchObject({ rank: 'D', inkToken: 'rank-d', glow: null });
    expect(at(0)).toMatchObject({ rank: 'E', inkToken: 'rank-e', glow: null });
  });

  // Low ranks read low by chroma and label, never luminance: every rank ink, D and E included, is
  // at least as legible as `--text-3` on every ground the glyph sits on.
  it('keeps every rank ink at --text-3 contrast or better', () => {
    const plate = oklchToRgb(LIGHTEST_PLATE_STOP.l, LIGHTEST_PLATE_STOP.c, LIGHTEST_PLATE_STOP.hue);
    const grounds = [tokenValue('surface-0'), tokenValue('surface-1'), plate];
    for (const ink of ['rank-s', 'rank-a', 'rank-b', 'rank-c', 'rank-d', 'rank-e'] as const) {
      for (const ground of grounds) {
        expect(ratioOf(tokenValue(ink), ground)).toBeGreaterThanOrEqual(
          ratioOf(tokenValue('text-3'), ground),
        );
      }
    }
  });
});
