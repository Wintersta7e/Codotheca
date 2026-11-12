/**
 * §43's copy module: every string the motivation layer renders. Its constraints are pinned by
 * shape — §39.10a's *rhythm, never streak* in every rendered string, §43.2's condensed block note
 * of at most two words and never a count, the figure order each formatter keeps, and §43.4's rise
 * line naming one transition's term from its receipt — and no string ships a placeholder.
 */
import { describe, expect, it } from 'vitest';

import type { MotivationReason, XpTerm } from '../../generated/protocol';
import { QUIET_LIST } from '../../shared/motivationQuiet';
import * as copy from './copy';

const reason = (over: Partial<MotivationReason> = {}): MotivationReason => ({
  transitions: 1,
  term: null,
  projectName: 'alpha',
  on: '2026-03-04',
  ...over,
});

/** Every string the module can render: its constants, and each formatter over sample inputs. */
function rendered(): [string, string][] {
  const out: [string, string][] = [];
  for (const [name, value] of Object.entries(copy)) {
    if (typeof value === 'string') out.push([name, value]);
    // The label tables and the words each accessible name carries.
    else if (typeof value === 'object') {
      for (const [key, entry] of Object.entries(value)) {
        if (typeof entry === 'string') out.push([`${name}.${key}`, entry]);
      }
    }
  }
  out.push(['XP_LINE_PARTIAL', copy.XP_LINE_PARTIAL(120, 3)]);
  out.push(['XP_LINE_COMPUTED', copy.XP_LINE_COMPUTED(120)]);
  out.push(['RHYTHM_LINE', copy.RHYTHM_LINE(3, 4, 2)]);
  out.push(['RHYTHM_LINE unset', copy.RHYTHM_LINE(0, null, null)]);
  out.push(['RHYTHM_SHORT', copy.RHYTHM_SHORT(3, 4, 2)]);
  out.push(['RHYTHM_SHORT unset', copy.RHYTHM_SHORT(0, null, null)]);
  out.push(['LINE_RISE one', copy.LINE_RISE(7, reason())]);
  out.push(['LINE_RISE several', copy.LINE_RISE(7, reason({ transitions: 4 }))]);
  out.push(['LINE_RISE none', copy.LINE_RISE(7, null)]);
  out.push(['LINE_COVERAGE_PARTIAL', copy.LINE_COVERAGE_PARTIAL(3)]);
  return out;
}

describe('the motivation copy module', () => {
  it('renders no string holding the word streak, in any case', () => {
    const strings = rendered();
    expect(strings.length).toBeGreaterThan(15);
    for (const [name, text] of strings) {
      expect(text.length, `${name} is empty`).toBeGreaterThan(0);
      expect(text.toLowerCase(), name).not.toContain('streak');
    }
  });

  it('ships words in every string, never a placeholder', () => {
    for (const [name, text] of rendered()) {
      expect(text, name).not.toContain('‹');
      expect(text, name).not.toContain('COPY');
    }
  });

  it('keeps the condensed block note to at most two words and no digit', () => {
    const words = copy.CONDENSED_LINE_WAITS.trim().split(/\s+/u);
    expect(words.length).toBeGreaterThan(0);
    expect(words.length).toBeLessThanOrEqual(2);
    expect(copy.CONDENSED_LINE_WAITS).not.toMatch(/[0-9]/u);
  });

  it('renders the figures a formatter is handed, and an unknown as U+2014, never zero', () => {
    expect(copy.XP_LINE_COMPUTED(120)).toContain('120');
    expect(copy.XP_LINE_PARTIAL(120, 3)).toMatch(/120.*3/u);
    expect(copy.RHYTHM_LINE(3, 4, 2)).toMatch(/3.*4.*2/u);
    const unset = copy.RHYTHM_LINE(5, null, null);
    expect(unset).toContain('—');
    expect(unset).not.toMatch(/\b0\b/u);
    expect(copy.RHYTHM_SHORT(3, 4, 2)).toMatch(/3.*4.*2/u);
    const shortUnset = copy.RHYTHM_SHORT(5, null, null);
    expect(shortUnset.match(/—/gu)).toHaveLength(2);
    expect(shortUnset).not.toMatch(/\b0\b/u);
  });

  it('names one transition from its receipt and counts several', () => {
    expect(copy.LINE_RISE(7, reason())).toContain('alpha');
    const several = copy.LINE_RISE(7, reason({ transitions: 4 }));
    expect(several).toContain('4');
    expect(several).not.toContain('alpha');
    expect(several).toBe('LEVEL 7 · FROM 4 NEW ENTRIES');
    expect(copy.LINE_RISE(7, null)).toBe('LEVEL 7');
  });

  it('names one transition by its term, then its project and its date, nulls dropped', () => {
    expect(copy.LINE_RISE(25, reason({ term: 'release', on: '2026-11-10' }))).toBe(
      'LEVEL 25 · RELEASE TAGGED · alpha · 2026-11-10',
    );
    const terms = Object.keys(copy.RISE_TERM_WORDS) as XpTerm[];
    expect(terms).toHaveLength(9);
    for (const term of terms) {
      const words = copy.RISE_TERM_WORDS[term];
      expect(words.length, term).toBeGreaterThan(0);
      expect(copy.LINE_RISE(7, reason({ term })), term).toBe(
        `LEVEL 7 · ${words} · alpha · 2026-03-04`,
      );
    }
    expect(copy.LINE_RISE(7, reason({ term: 'commitDay', projectName: null }))).toBe(
      'LEVEL 7 · COMMIT-DAY · 2026-03-04',
    );
    expect(copy.LINE_RISE(7, reason({ projectName: null, on: null }))).toBe('LEVEL 7');
  });

  it('counts a single project and a single week in the singular, every other count plural', () => {
    expect(copy.XP_LINE_PARTIAL(120, 1)).toBe(
      '120 XP TO NEXT LEVEL · 1 PROJECT STILL INDEXING · CAN ONLY GROW',
    );
    expect(copy.XP_LINE_PARTIAL(120, 3)).toContain('3 PROJECTS STILL INDEXING');
    expect(copy.XP_LINE_PARTIAL(120, 3)).toContain(copy.XP_LINE_COMPUTED(120));
    expect(copy.LINE_COVERAGE_PARTIAL(1)).toBe('1 PROJECT STILL INDEXING');
    expect(copy.RHYTHM_LINE(3, 4, 1)).toBe('3 OF 4 DAYS · BEST 1 WEEK');
    expect(copy.RHYTHM_LINE(3, 4, 6)).toBe('3 OF 4 DAYS · BEST 6 WEEKS');
    expect(copy.RHYTHM_LINE(null, null, null)).toBe('— OF — DAYS · BEST — WEEKS');
  });

  it('makes the quiet note the list read from its module', () => {
    for (const entry of QUIET_LIST) expect(copy.QUIET_SWITCH_NOTE).toContain(entry.label);
  });
});
