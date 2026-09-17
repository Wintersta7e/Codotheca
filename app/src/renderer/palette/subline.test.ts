import { describe, expect, it } from 'vitest';
import { makeProjectRow } from '../testing/projectRow.js';
import { paletteSubLine, subLineInputFor } from './subline.js';

const DAY = 86_400;
const NOW = 1_800_000_000;

describe('paletteSubLine', () => {
  it('reads sigil · branch · tail when every field is present', () => {
    expect(
      paletteSubLine({
        primaryLanguage: 'Rust',
        branch: 'main',
        lastTouchedAt: NOW - 2 * DAY,
        inSession: false,
        nowSecs: NOW,
        hasWorkingCopy: true,
      }),
    ).toBe('.rs · main · opened this month');
  });

  it('says in session while a session is open, whatever the clock says', () => {
    expect(
      paletteSubLine({
        primaryLanguage: 'Rust',
        branch: 'main',
        lastTouchedAt: NOW - 900 * DAY,
        inSession: true,
        nowSecs: NOW,
        hasWorkingCopy: true,
      }),
    ).toBe('.rs · main · in session');
  });

  it('cuts opened this month at exactly 30 days', () => {
    const at = (days: number): string =>
      paletteSubLine({
        primaryLanguage: null,
        branch: null,
        lastTouchedAt: NOW - days * DAY,
        inSession: false,
        nowSecs: NOW,
        hasWorkingCopy: true,
      });
    expect(at(30)).toBe('opened this month');
    expect(at(31)).toBe('1 months cold');
    expect(at(75)).toBe('3 months cold');
    expect(at(365)).toBe('12 months cold');
  });

  // Never render unknown as something. A row with no language and no branch is three
  // fields short of the design's line and renders the one field it has.
  it('drops an absent sigil or branch together with its separator', () => {
    expect(
      paletteSubLine({
        primaryLanguage: 'Haskell',
        branch: 'main',
        lastTouchedAt: NOW,
        inSession: false,
        nowSecs: NOW,
        hasWorkingCopy: true,
      }),
    ).toBe('main · opened this month');
    expect(
      paletteSubLine({
        primaryLanguage: 'Go',
        branch: null,
        lastTouchedAt: NOW,
        inSession: false,
        nowSecs: NOW,
        hasWorkingCopy: true,
      }),
    ).toBe('.go · opened this month');
  });

  it('never reads a negative age when a clock skews forward', () => {
    expect(
      paletteSubLine({
        primaryLanguage: null,
        branch: null,
        lastTouchedAt: NOW + 5 * DAY,
        inSession: false,
        nowSecs: NOW,
        hasWorkingCopy: true,
      }),
    ).toBe('opened this month');
  });
});

describe('subLineInputFor', () => {
  it('reads the projection fields §8.3a’s delta added for this line', () => {
    const row = makeProjectRow({
      primaryLanguage: 'Lua',
      branch: 'trunk',
      lastTouchedAt: NOW - DAY,
    });
    expect(subLineInputFor(row, true, NOW)).toEqual({
      primaryLanguage: 'Lua',
      branch: 'trunk',
      lastTouchedAt: NOW - DAY,
      inSession: true,
      nowSecs: NOW,
      hasWorkingCopy: true,
    });
  });
});

/**
 * AC-P2-23-6's unit half, now carrying §24.5's word. It does not satisfy either criterion on its
 * own — §23.9 requires the row to be rendered — but it pins the rule at the one place that owns
 * it. §23 left this seam for §24 and named p2-24 as the supplier; nothing here is a second field.
 */
describe('§24.5: the tail of a project with no working copy is `not cloned`', () => {
  const line = (hasWorkingCopy: boolean, over: Record<string, unknown> = {}): string =>
    paletteSubLine({
      primaryLanguage: 'Rust',
      branch: 'main',
      lastTouchedAt: NOW - 2 * DAY,
      inSession: false,
      nowSecs: NOW,
      hasWorkingCopy,
      ...over,
    });

  it('states the fact instead of computing an age, and leaves a located line alone', () => {
    expect(line(false)).toBe('.rs · main · not cloned');
    expect(line(true)).toBe('.rs · main · opened this month');
  });

  it('never renders an age, whatever the clock and whatever the session says', () => {
    for (const days of [0, 1, 30, 31, 400]) {
      expect(line(false, { lastTouchedAt: NOW - days * DAY })).toBe('.rs · main · not cloned');
    }
    // `in session` is the one tail word a not-cloned project could reach through a different
    // input, so it is asserted explicitly rather than left to the clock spread above.
    expect(line(false, { inSession: true })).toBe('.rs · main · not cloned');
  });

  it('is the whole line when the tail is the only field it had', () => {
    expect(
      paletteSubLine({
        primaryLanguage: null,
        branch: null,
        lastTouchedAt: NOW,
        inSession: false,
        nowSecs: NOW,
        hasWorkingCopy: false,
      }),
    ).toBe('not cloned');
  });

  it('reads the one predicate §23.1 names and never a second one', () => {
    expect(
      subLineInputFor(makeProjectRow({ primaryLocation: null }), false, NOW).hasWorkingCopy,
    ).toBe(false);
    expect(subLineInputFor(makeProjectRow(), false, NOW).hasWorkingCopy).toBe(true);
    // R92: §24 fills the seam §23 left; it adds no `hasLocation` beside it. A second field would
    // be one value stated twice over the predicate §23.1 says has exactly one expression.
    expect(Object.keys(subLineInputFor(makeProjectRow(), false, NOW)).sort()).toEqual([
      'branch',
      'hasWorkingCopy',
      'inSession',
      'lastTouchedAt',
      'nowSecs',
      'primaryLanguage',
    ]);
  });
});
