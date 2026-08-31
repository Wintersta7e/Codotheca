import { describe, expect, it } from 'vitest';
import type { Activity, ActivityWeek, LaneState } from '../../../generated/protocol';
import { activityFixture, NOW } from '../testFixtures';
import {
  AXIS_LABELS,
  commitAlpha,
  commitCell,
  COMMIT_DAYS_FULL,
  formatCommitDate,
  ledgerNote,
  LEGEND,
  sessionCell,
  SESSIONS_FULL,
  weekTooltip,
  WEEK_SLOTS,
} from './activityLanes';

const week = (over: Partial<ActivityWeek> = {}): ActivityWeek => ({
  weekStart: NOW,
  commitDays: 0,
  sessionCount: 0,
  sessionSeconds: 0,
  ...over,
});

const lanes = (
  commitDays: LaneState,
  sessions: LaneState,
  over: Partial<ActivityWeek> = {},
): Activity => activityFixture({ commitDays, sessions, weeks: [week(over)] });

describe('the slots', () => {
  it('is 26 weeks, and the axis names both ends', () => {
    expect(WEEK_SLOTS).toBe(26);
    expect(AXIS_LABELS).toEqual(['26 WEEKS AGO', 'THIS WEEK']);
  });
});

describe('the two lanes are normalised apart', () => {
  it('scales commit-days against seven days, the only denominator a week has', () => {
    expect(COMMIT_DAYS_FULL).toBe(7);
    expect(commitCell(week({ commitDays: 7 }), 'measured')).toEqual({
      kind: 'bar',
      heightPct: 100,
    });
    expect(commitCell(week({ commitDays: 14 }), 'measured')).toEqual({
      kind: 'bar',
      heightPct: 100,
    });
  });

  it('scales sessions against its own full scale, not the other lane’s', () => {
    expect(SESSIONS_FULL).toBe(5);
    expect(sessionCell(week({ sessionCount: 5 }), 'measured')).toEqual({
      kind: 'bar',
      heightPct: 100,
    });
    // Same input, different lane, different height: the two are not comparable by construction.
    expect(commitCell(week({ commitDays: 5 }), 'measured')).not.toEqual(
      sessionCell(week({ sessionCount: 5 }), 'measured'),
    );
  });

  it('fades the commit fill with the days in it', () => {
    expect(commitAlpha(0)).toBeCloseTo(0.4);
    expect(commitAlpha(7)).toBeCloseTo(0.95);
    expect(commitAlpha(9)).toBeCloseTo(0.95);
  });
});

describe('the three states, separated by the baseline', () => {
  it('draws a hairline for an indexed, measured zero', () => {
    expect(commitCell(week({ commitDays: 0 }), 'measured')).toEqual({ kind: 'hairline' });
    expect(sessionCell(week({ sessionCount: 0 }), 'measured')).toEqual({ kind: 'hairline' });
  });

  it('draws nothing at all for a week that was never computed', () => {
    expect(commitCell(week({ commitDays: null }), 'measured')).toEqual({ kind: 'blank' });
    expect(commitCell(week({ commitDays: 3 }), 'not_computed')).toEqual({ kind: 'blank' });
    expect(sessionCell(week({ sessionCount: null }), 'measured')).toEqual({ kind: 'blank' });
  });

  it('draws nothing for a shallow history, which is excluded permanently', () => {
    expect(commitCell(week({ commitDays: 4 }), 'shallow_excluded')).toEqual({ kind: 'blank' });
  });
});

describe('the header note', () => {
  it('states both ledgers and keeps them apart', () => {
    expect(ledgerNote(lanes('measured', 'measured', { sessionCount: 2 }))).toBe(
      'COMMIT-DAYS FROM HISTORY · SESSIONS FROM THIS INSTALL',
    );
  });

  it('says there are no sessions yet rather than drawing an empty lane silently', () => {
    expect(ledgerNote(lanes('measured', 'measured', { sessionCount: 0 }))).toBe(
      'COMMIT-DAYS FROM HISTORY · NO SESSIONS YET',
    );
  });

  it('names not-computed and shallow as the two reasons a lane can be blank', () => {
    expect(ledgerNote(lanes('not_computed', 'measured'))).toBe(
      'COMMIT-DAYS NOT YET COMPUTED · NO SESSIONS YET',
    );
    expect(ledgerNote(lanes('shallow_excluded', 'measured'))).toBe(
      'HISTORY IS SHALLOW — COMMIT-DAYS NOT COUNTED · NO SESSIONS YET',
    );
  });

  it('says the session lane is uncomputed rather than empty when nothing has counted it', () => {
    expect(ledgerNote(lanes('measured', 'not_computed'))).toBe(
      'COMMIT-DAYS FROM HISTORY · SESSIONS NOT YET COMPUTED',
    );
  });

  it('never says COMMITS FROM HISTORY, and never names a commit count', () => {
    const all = (['measured', 'not_computed', 'shallow_excluded'] as const)
      .map((s) => ledgerNote(lanes(s, 'measured')))
      .join(' ');
    expect(all).not.toContain('COMMITS FROM HISTORY');
    expect(all).not.toMatch(/COMMITS BY YOU|\bCOMMITS\b/);
  });
});

describe('the tooltip', () => {
  it('names the two ledgers separately and never a sum', () => {
    expect(
      weekTooltip(week({ commitDays: 4, sessionCount: 2 }), lanes('measured', 'measured')),
    ).toBe('4 commit-days · 2 sessions');
  });

  it('says not computed rather than zero for a lane that has no figure', () => {
    expect(
      weekTooltip(week({ commitDays: null, sessionCount: 2 }), lanes('measured', 'measured')),
    ).toBe('commit-days not computed · 2 sessions');
    expect(
      weekTooltip(week({ commitDays: 4, sessionCount: 1 }), lanes('shallow_excluded', 'measured')),
    ).toBe('commit-days not counted · 1 session');
  });
});

describe('the legend', () => {
  it('names the lanes as days and sessions, never as commits', () => {
    expect(LEGEND.map((l) => l.label)).toEqual(['COMMIT-DAYS', 'LAUNCHED SESSIONS']);
  });
});

describe('the commit date', () => {
  /**
   * NOW is 2027-01-15T08:00Z, so the zone shift is proved with offsets that actually roll the
   * date: no real positive offset rolls 08:00Z forward, which is why the plan's `13 * 60`
   * expectation of `2027-01-16` cannot be produced by any correct implementation.
   */
  it('reads in the commit’s own zone, so a listing does not drift with the reader', () => {
    expect(formatCommitDate(NOW, 0)).toBe('2027-01-15');
    expect(formatCommitDate(NOW, -12 * 60)).toBe('2027-01-14');
    // 2027-01-14T23:00Z, two hours east: the same instant is already the next day there.
    expect(formatCommitDate(NOW - 9 * 3600, 0)).toBe('2027-01-14');
    expect(formatCommitDate(NOW - 9 * 3600, 2 * 60)).toBe('2027-01-15');
  });
});
