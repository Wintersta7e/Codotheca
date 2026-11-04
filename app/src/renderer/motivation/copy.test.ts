/**
 * §43's copy module: every string the motivation layer renders, each export one ‹COPY› slot. Two
 * constraints are checkable without the user's words — §39.10a's *rhythm, never streak* in every
 * rendered string, and §43.2's condensed block note of at most two words and never a count.
 */
import { describe, expect, it } from 'vitest';

import type { MotivationReason } from '../../generated/protocol';
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
  }
  out.push(['XP_LINE_PARTIAL', copy.XP_LINE_PARTIAL(120, 3)]);
  out.push(['XP_LINE_COMPUTED', copy.XP_LINE_COMPUTED(120)]);
  out.push(['RHYTHM_LINE', copy.RHYTHM_LINE(3, 4, 2)]);
  out.push(['RHYTHM_LINE unset', copy.RHYTHM_LINE(0, null, null)]);
  out.push(['LINE_RISE one', copy.LINE_RISE(7, reason())]);
  out.push(['LINE_RISE several', copy.LINE_RISE(7, reason({ transitions: 4 }))]);
  out.push(['LINE_RISE none', copy.LINE_RISE(7, null)]);
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
  });

  it('names one transition from its receipt and counts several', () => {
    expect(copy.LINE_RISE(7, reason())).toContain('alpha');
    const several = copy.LINE_RISE(7, reason({ transitions: 4 }));
    expect(several).toContain('4');
    expect(several).not.toContain('alpha');
  });

  it('makes the quiet note the list read from its module', () => {
    for (const entry of QUIET_LIST) expect(copy.QUIET_SWITCH_NOTE).toContain(entry.label);
  });
});
