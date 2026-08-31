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
      }),
    ).toBe('main · opened this month');
    expect(
      paletteSubLine({
        primaryLanguage: 'Go',
        branch: null,
        lastTouchedAt: NOW,
        inSession: false,
        nowSecs: NOW,
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
    });
  });
});
