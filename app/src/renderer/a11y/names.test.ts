import { describe, expect, it } from 'vitest';
import type { ConditionSignal } from '../../generated/protocol';
import {
  BANNED_NAME_WORDS,
  CARD_ROLE,
  COMPLETION_NOT_COMPUTED_NAME,
  GRID_ROLE,
  GRID_ROW_ROLE,
  SCAN_COUNT_ROLE,
  SKIP_AHEAD_LABEL,
  conditionDotName,
  eraChevronName,
  pinControlName,
  statesAColour,
} from './names';

const SIGNALS: readonly ConditionSignal[] = [
  'live',
  'idle',
  'dormant',
  'neglected',
  'abandoned',
  'offline',
  'empty',
];

describe('the condition dot is named by its word', () => {
  it('names the enum member verbatim', () => {
    expect(conditionDotName('dormant')).toBe('Condition: dormant');
    expect(conditionDotName('neglected')).toBe('Condition: neglected');
    expect(conditionDotName('abandoned')).toBe('Condition: abandoned');
  });

  it('names every member of the enum and nothing outside it', () => {
    for (const signal of SIGNALS) expect(conditionDotName(signal)).toBe(`Condition: ${signal}`);
  });

  it('is null when the signal is NULL, because no dot is drawn at all', () => {
    expect(conditionDotName(null)).toBeNull();
  });

  it('never states a colour, on any member', () => {
    for (const signal of SIGNALS) {
      expect(statesAColour(conditionDotName(signal) ?? '')).toBe(false);
    }
  });

  it('never uses the design band vocabulary the spec forbids as a string', () => {
    const all = SIGNALS.map((s) => conditionDotName(s) ?? '').join(' ');
    for (const banned of ['warm', 'cooling', 'blueprint']) expect(all).not.toContain(banned);
  });
});

describe('the pin control is named by its action and its subject', () => {
  it('names the act, not the shape', () => {
    expect(pinControlName('Aurora', false)).toBe('Pin Aurora');
    expect(pinControlName('Aurora', true)).toBe('Unpin Aurora');
  });

  it('never names the two rectangles', () => {
    const both = `${pinControlName('X', true)} ${pinControlName('X', false)}`;
    for (const shape of ['bar', 'shaft', 'rectangle', 'silhouette', 'icon']) {
      expect(both.toLowerCase()).not.toContain(shape);
    }
  });
});

describe('the frame and the rank glyph are named as an absence', () => {
  it('says the completion is not computed, not that the tier is unknown', () => {
    expect(COMPLETION_NOT_COMPUTED_NAME).toBe('Completion not computed');
    expect(statesAColour(COMPLETION_NOT_COMPUTED_NAME)).toBe(false);
    expect(COMPLETION_NOT_COMPUTED_NAME).not.toMatch(/\b0\b|zero|none/i);
  });
});

describe('the era chevron is named by the header it collapses', () => {
  it('takes the summary line verbatim', () => {
    expect(eraChevronName('2019 and earlier · 212 projects')).toBe(
      '2019 and earlier · 212 projects',
    );
  });
});

describe('the roles nothing else may pick', () => {
  it('fixes the card, the grid and its rows', () => {
    expect(CARD_ROLE).toBe('gridcell');
    expect(GRID_ROLE).toBe('grid');
    expect(GRID_ROW_ROLE).toBe('row');
  });

  it('fixes the scan count as a live region and the escape control label', () => {
    expect(SCAN_COUNT_ROLE).toBe('status');
    expect(SKIP_AHEAD_LABEL).toBe('SKIP AHEAD');
  });
});

describe('the colour guard', () => {
  it('catches a name that leaks the paint', () => {
    expect(statesAColour('Condition: blue')).toBe(true);
    expect(statesAColour('Gold tier')).toBe(true);
    expect(statesAColour('Amber warning')).toBe(true);
    expect(BANNED_NAME_WORDS.length).toBeGreaterThan(5);
  });

  it('does not catch a legitimate name that merely contains a substring', () => {
    // "goldfinch" is not "gold"; word boundaries matter or the guard fires on real prose.
    expect(statesAColour('Unpin goldfinch')).toBe(false);
  });
});
