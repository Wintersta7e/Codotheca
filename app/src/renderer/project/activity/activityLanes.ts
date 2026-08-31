/**
 * §8.5.5. The two ledgers are drawn apart, and here that is a geometric instruction and not only
 * an arithmetic one: each week is two adjacent sub-columns sharing the slot, each with its own
 * baseline and its own normalisation. Stacked, the eye reads the column as a total — and with
 * different denominators that total encodes nothing at all.
 *
 * Nothing in this module adds a commit-day to a session, and nothing in it produces a commit
 * count: J4 produces days, the schema stores no count, and never-reward-volume forbids acquiring
 * one.
 */
import type { Activity, ActivityWeek, LaneState } from '../../../generated/protocol';

export const WEEK_SLOTS = 26;

/** A week has seven days in it. That is the commit lane's only honest denominator. */
export const COMMIT_DAYS_FULL = 7;

/** The session lane's own full scale. It is not comparable to the lane beside it, by design. */
export const SESSIONS_FULL = 5;

export type LaneCell =
  { kind: 'bar'; heightPct: number } | { kind: 'hairline' } | { kind: 'blank' };

/**
 * The baseline is what separates the three states. A measured zero is a hairline — a flat,
 * complete record of doing nothing — and a week nothing computed is blank. Collapsing the two is
 * the single easiest place in the product to render unknown as zero.
 */
function cell(value: number | null, state: LaneState, full: number): LaneCell {
  if (state !== 'measured' || value === null) return { kind: 'blank' };
  if (value <= 0) return { kind: 'hairline' };
  return { kind: 'bar', heightPct: Math.round(Math.min(1, value / full) * 100) };
}

export function commitCell(week: ActivityWeek, state: LaneState): LaneCell {
  return cell(week.commitDays, state, COMMIT_DAYS_FULL);
}

export function sessionCell(week: ActivityWeek, state: LaneState): LaneCell {
  return cell(week.sessionCount, state, SESSIONS_FULL);
}

/** §8.5.5: the project jewel at `0.4 + min(1, days / 7) * 0.55`. */
export function commitAlpha(days: number): number {
  return 0.4 + Math.min(1, Math.max(0, days) / COMMIT_DAYS_FULL) * 0.55;
}

function commitHalf(state: LaneState): string {
  if (state === 'not_computed') return 'COMMIT-DAYS NOT YET COMPUTED';
  if (state === 'shallow_excluded') return 'HISTORY IS SHALLOW — COMMIT-DAYS NOT COUNTED';
  return 'COMMIT-DAYS FROM HISTORY';
}

function sessionHalf(activity: Activity): string {
  if (activity.sessions !== 'measured') return 'SESSIONS NOT YET COMPUTED';
  const any = activity.weeks.some((w) => (w.sessionCount ?? 0) > 0);
  return any ? 'SESSIONS FROM THIS INSTALL' : 'NO SESSIONS YET';
}

/** The note is what keeps the two ledgers apart, and is read before either lane is. */
export function ledgerNote(activity: Activity): string {
  return `${commitHalf(activity.commitDays)} · ${sessionHalf(activity)}`;
}

function commitPhrase(week: ActivityWeek, state: LaneState): string {
  if (state === 'shallow_excluded') return 'commit-days not counted';
  if (state !== 'measured' || week.commitDays === null) return 'commit-days not computed';
  return `${String(week.commitDays)} commit-day${week.commitDays === 1 ? '' : 's'}`;
}

function sessionPhrase(week: ActivityWeek, state: LaneState): string {
  if (state !== 'measured' || week.sessionCount === null) return 'sessions not computed';
  return `${String(week.sessionCount)} session${week.sessionCount === 1 ? '' : 's'}`;
}

export function weekTooltip(week: ActivityWeek, activity: Activity): string {
  return `${commitPhrase(week, activity.commitDays)} · ${sessionPhrase(week, activity.sessions)}`;
}

export const AXIS_LABELS = ['26 WEEKS AGO', 'THIS WEEK'] as const;

export const LEGEND = [
  { id: 'commitDays', label: 'COMMIT-DAYS' },
  { id: 'sessions', label: 'LAUNCHED SESSIONS' },
] as const;

/** The commit's own zone, from `CommitRef.tzOffsetMin`, so a listing does not move with a reader. */
export function formatCommitDate(atSecs: number, tzOffsetMin: number): string {
  const shifted = new Date((atSecs + tzOffsetMin * 60) * 1000);
  const y = shifted.getUTCFullYear();
  const m = String(shifted.getUTCMonth() + 1).padStart(2, '0');
  const d = String(shifted.getUTCDate()).padStart(2, '0');
  return `${String(y)}-${m}-${d}`;
}
