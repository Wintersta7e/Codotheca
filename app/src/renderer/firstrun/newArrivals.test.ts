import { describe, expect, it } from 'vitest';
import type { ProjectId } from '../../generated/protocol';
import {
  NEW_ARRIVALS_QUERY,
  type NewArrivalRow,
  acknowledges,
  isNewArrival,
  newArrivals,
  newArrivalsNotice,
  sinceLabel,
} from './newArrivals';

const FIRST_RUN = 1_700_000_000;
/** Tuesday 25 August 2026, 09:00 local — the reference "now" for every day label below. */
const NOW = new Date(2026, 7, 25, 9, 0, 0).getTime() / 1000;
const id = (n: number): ProjectId => n as unknown as ProjectId;

const row = (over: Partial<NewArrivalRow> = {}): NewArrivalRow => ({
  id: id(1),
  createdAt: FIRST_RUN + 86_400,
  acknowledgedAt: null,
  ...over,
});

describe('§10.5a: created after first run and never opened', () => {
  it('is new when both halves hold', () => {
    expect(isNewArrival(row(), FIRST_RUN)).toBe(true);
  });

  it('is not new once it has been acknowledged', () => {
    expect(isNewArrival(row({ acknowledgedAt: FIRST_RUN + 90_000 }), FIRST_RUN)).toBe(false);
  });

  it('is not new when it predates the boundary', () => {
    expect(isNewArrival(row({ createdAt: FIRST_RUN - 1 }), FIRST_RUN)).toBe(false);
  });

  it('reads the boundary as exclusive, so a project indexed by first run is not new', () => {
    // Keyed on `created_at` alone every project is new on day one; the boundary is what stops
    // the entire first scan arriving as a wall of chips.
    expect(isNewArrival(row({ createdAt: FIRST_RUN }), FIRST_RUN)).toBe(false);
  });
});

describe('while first run has not completed', () => {
  it('is never new, because nothing is new relative to what the user had', () => {
    expect(isNewArrival(row(), null)).toBe(false);
    expect(isNewArrival(row({ createdAt: FIRST_RUN + 1_000_000 }), null)).toBe(false);
  });

  it('filters and summarises to nothing, so no chip and no row can appear', () => {
    expect(newArrivals([row(), row({ id: id(2) })], null)).toEqual([]);
    expect(newArrivalsNotice([row()], null, NOW)).toBeNull();
  });
});

// §10.5a: never derived from `last_interaction_at`, which counts reflog activity performed
// outside the app. A commit made in a terminal must leave the chip standing.
describe('what does and does not acknowledge', () => {
  it('ignores activity outside the app entirely', () => {
    const touchedOutside = { ...row(), lastInteractionAt: NOW } as NewArrivalRow;
    expect(isNewArrival(touchedOutside, FIRST_RUN)).toBe(true);
  });

  // Cleared by opening the project page, or launching. Not by Peek: §8.4's Peek is the triage
  // mechanism, and clearing on it lets one arrow-key run down a column erase the whole batch.
  it('counts opening and launching, and never peeking', () => {
    expect(acknowledges('openedProjectPage')).toBe(true);
    expect(acknowledges('launched')).toBe(true);
    expect(acknowledges('peeked')).toBe(false);
  });
});

describe('§10.5a: the dismissible row', () => {
  const at = (day: number): number => new Date(2026, 7, day, 8, 0, 0).getTime() / 1000;

  it('states the count and the day the batch starts from', () => {
    const notice = newArrivalsNotice(
      [
        row({ createdAt: at(23) }),
        row({ id: id(2), createdAt: at(24) }),
        row({ id: id(3), createdAt: at(25) }),
      ],
      FIRST_RUN,
      NOW,
    );
    expect(notice).not.toBeNull();
    expect(notice?.count).toBe(3);
    expect(notice?.since).toBe('Sunday');
    expect(notice?.text).toBe('3 new · since Sunday');
    expect(notice?.query).toBe(NEW_ARRIVALS_QUERY);
    expect(NEW_ARRIVALS_QUERY).toBe('is:new');
  });

  it('counts one as one', () => {
    expect(newArrivalsNotice([row({ createdAt: at(24) })], FIRST_RUN, NOW)?.text).toBe(
      '1 new · since yesterday',
    );
  });

  it('renders no row when nothing arrived', () => {
    expect(newArrivalsNotice([row({ createdAt: FIRST_RUN - 10 })], FIRST_RUN, NOW)).toBeNull();
    expect(newArrivalsNotice([], FIRST_RUN, NOW)).toBeNull();
  });

  it('takes the earliest arrival, not the newest, so the row spans the whole batch', () => {
    const notice = newArrivalsNotice(
      [row({ createdAt: at(25) }), row({ id: id(2), createdAt: at(23) })],
      FIRST_RUN,
      NOW,
    );
    expect(notice?.since).toBe('Sunday');
  });

  it('counts only the arrivals, never the rows it was handed', () => {
    const notice = newArrivalsNotice(
      [row({ createdAt: at(24) }), row({ id: id(2), createdAt: at(23), acknowledgedAt: NOW })],
      FIRST_RUN,
      NOW,
    );
    expect(notice?.count).toBe(1);
    expect(notice?.since).toBe('yesterday');
  });
});

// The weekday table is fixed rather than `Intl`, so `3 new · since mardi` cannot appear on an
// English shelf and a test cannot pass on one machine and fail on another.
describe('the day the batch starts from, named the way a reader would name it', () => {
  const at = (day: number): number => new Date(2026, 7, day, 8, 0, 0).getTime() / 1000;

  it('names today, yesterday, a weekday inside the week and a date outside it', () => {
    expect(sinceLabel(at(25), NOW)).toBe('today');
    expect(sinceLabel(at(24), NOW)).toBe('yesterday');
    expect(sinceLabel(at(23), NOW)).toBe('Sunday');
    expect(sinceLabel(at(19), NOW)).toBe('Wednesday');
    expect(sinceLabel(at(18), NOW)).toBe('18 August');
    expect(sinceLabel(new Date(2025, 10, 4, 8, 0, 0).getTime() / 1000, NOW)).toBe(
      '4 November 2025',
    );
  });

  // Calendar days, not 24-hour blocks: 23:30 last night is `yesterday` at 00:30, not `today`.
  it('counts calendar days from local midnight', () => {
    const lateLastNight = new Date(2026, 7, 24, 23, 30, 0).getTime() / 1000;
    const earlyToday = new Date(2026, 7, 25, 0, 30, 0).getTime() / 1000;
    expect(sinceLabel(lateLastNight, earlyToday)).toBe('yesterday');
  });
});
