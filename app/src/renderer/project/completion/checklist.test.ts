import { describe, expect, it } from 'vitest';
import type { CompletionCheckRow, UnknownReason } from '../../../generated/protocol';
import { CHECK_LABELS, markFor, noteFor } from './checklist';

const row = (over: Partial<CompletionCheckRow> = {}): CompletionCheckRow => ({
  key: 'readme',
  state: 'pass',
  userNa: null,
  unknownReason: null,
  observedAt: 1_700_000_000,
  ...over,
});

describe('the four marks', () => {
  it('draws a filled tick for pass and a hollow box for fail', () => {
    expect(markFor(row({ state: 'pass' }))).toEqual({ glyph: '▣', hollow: false });
    expect(markFor(row({ state: 'fail' }))).toEqual({ glyph: '□', hollow: false });
  });

  it('draws unknown hollow-ringed and NEVER as a dark tick', () => {
    const mark = markFor(row({ state: 'unknown' }));
    expect(mark.glyph).toBe('◌');
    expect(mark.hollow).toBe(true);
    // A dark tick is `fail`, and that is the invariant.
    expect(mark.glyph).not.toBe(markFor(row({ state: 'fail' })).glyph);
    expect(mark.glyph).not.toBe(markFor(row({ state: 'pass' })).glyph);
  });

  it('draws na as a dash, distinct from all three', () => {
    const glyphs = (['pass', 'fail', 'unknown', 'na'] as const).map(
      (state) => markFor(row({ state })).glyph,
    );
    expect(new Set(glyphs).size).toBe(4);
    expect(markFor(row({ state: 'na' })).glyph).toBe('–');
  });
});

describe('the na split tells the user’s decision from the app’s', () => {
  it('reads MARKED NOT APPLICABLE only for a stored ruling', () => {
    expect(noteFor(row({ state: 'na', userNa: true }))).toBe('MARKED NOT APPLICABLE');
    // Both halves, because one alone passes against a single string.
    expect(noteFor(row({ state: 'na', userNa: null }))).toBe('NOT APPLICABLE');
    // `false` is an override of a proposal, so a row in `na` with it is not the user's ruling.
    expect(noteFor(row({ state: 'na', userNa: false }))).toBe('NOT APPLICABLE');
  });

  it('renders no note for a state that was evaluated', () => {
    expect(noteFor(row({ state: 'pass' }))).toBeNull();
    expect(noteFor(row({ state: 'fail' }))).toBeNull();
  });
});

describe('the unknown note names the reason, never a forge that was not involved', () => {
  it('gives a locally evaluable check a note that does not mention a forge', () => {
    const local: readonly UnknownReason[] = ['notRead', 'notObserved', 'notRunYet', 'unreachable'];
    for (const reason of local) {
      const note = noteFor(row({ key: 'license', state: 'unknown', unknownReason: reason }));
      expect(note).not.toBeNull();
      expect(note).not.toContain('GITHUB');
      expect(note).not.toContain('ACCOUNT');
    }
    // The case the prototype got wrong: a budget exceedance on a LICENSE has nothing to do with
    // an account.
    expect(noteFor(row({ key: 'license', state: 'unknown', unknownReason: 'notRead' }))).toBe(
      'UNKNOWN · NOT READ',
    );
  });

  it('keeps the three self-resolving states apart', () => {
    // `notObserved` never resolves on its own; `notRunYet` resolves at the next sweep and
    // `unreachable` when the drive comes back. One string for all three would make a
    // self-resolving state read as a permanent one.
    const notes = (['notObserved', 'notRunYet', 'unreachable'] as const).map((reason) =>
      noteFor(row({ state: 'unknown', unknownReason: reason })),
    );
    expect(new Set(notes).size).toBe(3);
  });

  it('says UNKNOWN and invents no cause for a row carrying none', () => {
    expect(noteFor(row({ state: 'unknown', unknownReason: null }))).toBe('UNKNOWN');
  });
});

describe('the ten labels have one owner', () => {
  it('names every key exactly once', () => {
    const labels = Object.values(CHECK_LABELS);
    expect(labels).toHaveLength(10);
    expect(new Set(labels).size).toBe(10);
  });
});
