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
 * AC-P2-23-6's unit half. It does not satisfy the criterion on its own — §23.9 requires the row
 * to be rendered — but it pins the rule at the one place that owns it.
 */
describe('§23.4: no tail for a project with no working copy', () => {
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

  it('omits the tail field and renders the rest of the line', () => {
    expect(line(false)).toBe('.rs · main');
    expect(line(true)).toBe('.rs · main · opened this month');
  });

  it('omits it whatever the clock and whatever the session says', () => {
    for (const days of [0, 1, 30, 31, 400]) {
      expect(line(false, { lastTouchedAt: NOW - days * DAY })).toBe('.rs · main');
    }
    expect(line(false, { inSession: true })).toBe('.rs · main');
  });

  it('renders nothing at all when the tail is the only field it had', () => {
    expect(
      paletteSubLine({
        primaryLanguage: null,
        branch: null,
        lastTouchedAt: NOW,
        inSession: false,
        nowSecs: NOW,
        hasWorkingCopy: false,
      }),
    ).toBe('');
  });

  it('reads the one predicate §23.1 names and never a second one', () => {
    expect(
      subLineInputFor(makeProjectRow({ primaryLocation: null }), false, NOW).hasWorkingCopy,
    ).toBe(false);
    expect(subLineInputFor(makeProjectRow(), false, NOW).hasWorkingCopy).toBe(true);
  });
});
