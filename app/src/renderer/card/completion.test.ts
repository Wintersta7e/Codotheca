import { describe, expect, it } from 'vitest';
import { LADDER_RUNGS, RUNG_TOKENS, tokenValue } from '../theme/tokens';
import { densityStep } from './geometry';
import {
  ARCHIVED_GLASS,
  EM_DASH,
  GOLD_NOTCH,
  type RungInput,
  frameToken,
  paintsLadderRung,
  rungFor,
  scoreText,
  uncomputedRank,
} from './completion';

// A located project: `hasWorkingCopy` is set beside `isReference`, because §23.5 decides the
// frame from the pair and a fixture that carried only one would be describing half a row.
const uncomputed = {
  completionLit: null,
  isReference: false,
  hasWorkingCopy: true,
  density: 186,
};

describe('the grid card states the absence, because the slot is drawn anyway', () => {
  it('takes the unknown frame and cuts the gap in its top edge', () => {
    const rank = uncomputedRank('gridCard', uncomputed);
    expect(rank?.frameToken).toBe('unknown');
    expect(rank?.gap).toEqual({
      left: '39%',
      top: '-1px',
      width: '22%',
      height: '3px',
      fillToken: 'surface-1',
    });
  });

  it('draws U+2014 in --unknown-ink at the density glyph size, labelled NOT COMPUTED', () => {
    const rank = uncomputedRank('gridCard', uncomputed);
    expect(rank?.glyph).toBe('—');
    expect(EM_DASH).toBe('—');
    expect(rank?.glyphInkToken).toBe('unknown-ink');
    expect(rank?.glyphPx).toBe(densityStep(186).glyphPx);
    expect(uncomputedRank('gridCard', { ...uncomputed, density: 140 })?.glyphPx).toBe(30);
    expect(uncomputedRank('gridCard', { ...uncomputed, density: 260 })?.glyphPx).toBe(38);
    expect(rank?.label).toBe('NOT COMPUTED');
    expect(rank?.labelPx).toBe(7);
    expect(rank?.labelInkToken).toBe('text-3');
  });

  it('names the absence in the accessibility tree, not the colour of the frame', () => {
    expect(uncomputedRank('gridCard', uncomputed)?.accessibleName).toBe('Completion not computed');
  });
});

describe('the hero states it once, at its own sizes', () => {
  it('uses the hero glyph, label and gap fill', () => {
    const rank = uncomputedRank('hero', uncomputed);
    expect(rank?.glyphPx).toBe(64);
    expect(rank?.label).toBe('RANK NOT COMPUTED');
    expect(rank?.labelPx).toBe(7.5);
    expect(rank?.gap.fillToken).toBe('surface-0');
  });

  it('ignores density entirely — the hero is one size', () => {
    expect(uncomputedRank('hero', { ...uncomputed, density: 140 })?.glyphPx).toBe(64);
    expect(uncomputedRank('hero', { ...uncomputed, density: 400 })?.glyphPx).toBe(64);
  });
});

describe('every other readout draws no element at all', () => {
  it('is null on the list row and the palette row', () => {
    expect(uncomputedRank('listRow', uncomputed)).toBeNull();
    expect(uncomputedRank('paletteRow', uncomputed)).toBeNull();
  });

  it('is null on every surface once completion has a value, because this is the unknown case', () => {
    for (const surface of ['gridCard', 'hero', 'listRow', 'paletteRow'] as const) {
      expect(uncomputedRank(surface, { ...uncomputed, completionLit: 0 })).toBeNull();
      expect(uncomputedRank(surface, { ...uncomputed, completionLit: 7 })).toBeNull();
    }
  });
});

describe('the frame is decided above the ladder', () => {
  it('is tier-ref for a reference project and unknown for everything else', () => {
    expect(frameToken({ isReference: true, hasWorkingCopy: true })).toBe('tier-ref');
    expect(frameToken({ isReference: false, hasWorkingCopy: true })).toBe('unknown');
    expect(uncomputedRank('gridCard', { ...uncomputed, isReference: true })?.frameToken).toBe(
      'tier-ref',
    );
  });

  it('cuts the gap for a reference project too — completion is uncomputed there as well', () => {
    expect(uncomputedRank('gridCard', { ...uncomputed, isReference: true })?.gap.width).toBe('22%');
  });
});

describe('no surface may paint a rung', () => {
  it('rejects all six, plain and silver included', () => {
    for (const rung of LADDER_RUNGS) expect(paintsLadderRung(rung)).toBe(true);
    expect(LADDER_RUNGS).toHaveLength(6);
    expect(paintsLadderRung('#1e262e')).toBe(false);
    expect(paintsLadderRung('#232a31')).toBe(false);
  });

  it('is case-insensitive, because a stylesheet may upper-case a hex', () => {
    expect(paintsLadderRung('#E8C268')).toBe(true);
  });

  it('never emits one itself, on any surface, at any density', () => {
    for (const surface of ['gridCard', 'hero'] as const) {
      for (const density of [140, 186, 260]) {
        const rank = uncomputedRank(surface, { ...uncomputed, density });
        const emitted = JSON.stringify(rank);
        for (const rung of LADDER_RUNGS) expect(emitted).not.toContain(rung);
      }
    }
  });
});

describe('the gap and the notch mean opposite things and stay distinguishable', () => {
  it('differs in width and position', () => {
    const gap = uncomputedRank('gridCard', uncomputed)?.gap;
    expect(GOLD_NOTCH).toEqual({ right: '11px', top: '-1px', width: '9px', height: '3px' });
    expect(gap?.width).not.toBe(GOLD_NOTCH.width);
    expect(Object.keys(GOLD_NOTCH)).toContain('right');
    expect(Object.keys(gap ?? {})).toContain('left');
  });
});

describe('archived contributes the glass overlay and nothing else', () => {
  it('carries no ladder colour and no frame override', () => {
    expect(ARCHIVED_GLASS).toContain('linear-gradient(122deg');
    for (const rung of LADDER_RUNGS) expect(ARCHIVED_GLASS).not.toContain(rung);
    expect(uncomputedRank('gridCard', uncomputed)?.frameToken).toBe('unknown');
  });
});

// ---------------------------------------------------------------------------------------------
// [p3] §31.1c — the ladder
// ---------------------------------------------------------------------------------------------

const scored = (lit: number, evaluable: number, over: Partial<RungInput> = {}): RungInput => ({
  completionLit: lit,
  completionApplicable: evaluable,
  isReference: false,
  hasWorkingCopy: true,
  isArchived: false,
  ...over,
});

describe('the ladder reads lit over evaluable and nothing else', () => {
  it('walks the table in the order it is written', () => {
    expect(rungFor(scored(10, 10))?.rung).toBe('gold');
    expect(rungFor(scored(9, 10))?.rung).toBe('brass');
    expect(rungFor(scored(8, 10))?.rung).toBe('brass');
    expect(rungFor(scored(7, 10))?.rung).toBe('steel');
    expect(rungFor(scored(5, 10))?.rung).toBe('steel');
    expect(rungFor(scored(4, 10))?.rung).toBe('plain');
    expect(rungFor(scored(0, 10))?.rung).toBe('plain');
  });

  it('gives an archived project its own gold and silver everywhere else', () => {
    expect(rungFor(scored(10, 10, { isArchived: true }))?.rung).toBe('goldArchived');
    // Silver sits BELOW `pct >= 1`, so an archived project at 9/10 is silver and not brass.
    expect(rungFor(scored(9, 10, { isArchived: true }))?.rung).toBe('silver');
    expect(rungFor(scored(0, 10, { isArchived: true }))?.rung).toBe('silver');
  });

  it('answers null above the ladder, where §7.7a owns the frame', () => {
    expect(rungFor(scored(10, 10, { isReference: true }))).toBeNull();
    expect(rungFor(scored(10, 10, { hasWorkingCopy: false }))).toBeNull();
    expect(rungFor({ ...scored(10, 10), completionLit: null })).toBeNull();
    expect(rungFor({ ...scored(10, 10), completionApplicable: null })).toBeNull();
    // Zero evaluable is NotComputed wearing a number, not a measured zero.
    expect(rungFor(scored(0, 0))).toBeNull();
  });

  it('names a token for every rung and never a hex', () => {
    for (const input of [
      scored(10, 10),
      scored(10, 10, { isArchived: true }),
      scored(9, 10, { isArchived: true }),
      scored(9, 10),
      scored(6, 10),
      scored(1, 10),
    ]) {
      const paint = rungFor(input);
      expect(paint).not.toBeNull();
      expect(paint?.frameToken).not.toMatch(/^#/u);
      expect(paint?.inkToken).not.toMatch(/^#/u);
      // Every rung frame resolves to a member of the guard's own set.
      expect(paintsLadderRung(tokenValue(paint?.frameToken ?? 'tier-plain'))).toBe(true);
    }
  });
});

describe('the notch fires on evaluable, never on the N/A count', () => {
  it('notches gold at 8/8 with two unknown and leaves 10/10 plain', () => {
    // The design's own live fixture row: eight pass, zero fail, zero na, two unknown.
    const eightOfEight = rungFor(scored(8, 8));
    expect(eightOfEight?.rung).toBe('gold');
    expect(eightOfEight?.notched).toBe(true);
    expect(rungFor(scored(10, 10))?.notched).toBe(false);
  });

  it('notches a shrunk denominator whatever shrank it', () => {
    // Six N/A and four evaluable reaches the same notch as two unknown, which is the whole of
    // §31.1c's rule: the notch is about the DENOMINATOR, not about why it moved.
    expect(rungFor(scored(4, 4))?.notched).toBe(true);
    expect(rungFor(scored(7, 7, { isArchived: true }))?.notched).toBe(true);
  });

  it('never notches a rung below gold, which already carries its denominator', () => {
    expect(rungFor(scored(7, 8))?.notched).toBe(false);
    expect(rungFor(scored(4, 9))?.notched).toBe(false);
  });
});

describe('the score is a fraction, never a percentage and never a bare numerator', () => {
  it('renders lit over evaluable', () => {
    expect(scoreText(8, 8)).toBe('8/8');
    expect(scoreText(0, 10)).toBe('0/10');
  });

  it('renders nothing rather than a zero when either half is uncomputed', () => {
    expect(scoreText(null, 10)).toBeNull();
    expect(scoreText(8, null)).toBeNull();
    expect(scoreText(null, null)).toBeNull();
    // Zero evaluable is NotComputed, and a `0/0` would be a fraction over a denominator nobody
    // measured.
    expect(scoreText(0, 0)).toBeNull();
  });

  it('carries no percent sign at any value', () => {
    for (let lit = 0; lit <= 10; lit += 1) {
      expect(scoreText(lit, 10)).not.toContain('%');
    }
  });
});

describe('the notch and the gap mean opposite things', () => {
  it('keeps them distinguishable by width and position', () => {
    const gap = uncomputedRank('gridCard', uncomputed)?.gap;
    expect(GOLD_NOTCH.right).toBe('11px');
    expect(GOLD_NOTCH.width).toBe('9px');
    expect(gap?.left).toBe('39%');
    expect(gap?.width).toBe('22%');
    // A computed project draws no gap at all, which is what makes "never both" structural.
    expect(uncomputedRank('gridCard', { ...uncomputed, completionLit: 8 })).toBeNull();
  });
});

describe('the ladder set has one owner', () => {
  it('derives LADDER_RUNGS from the token map rather than from six literals', () => {
    expect(LADDER_RUNGS).toHaveLength(6);
    expect(LADDER_RUNGS).toEqual(RUNG_TOKENS.map((name) => tokenValue(name)));
    // Archived gold and silver are in the ladder and are NOT `--silver`, which §33 reads for
    // cobwebs — the collision R130/F14 names.
    expect(LADDER_RUNGS).toContain(tokenValue('tier-gold-archived'));
    expect(LADDER_RUNGS).toContain(tokenValue('tier-silver'));
    expect(tokenValue('tier-silver')).toBe(tokenValue('silver'));
    expect(RUNG_TOKENS).not.toContain('silver');
  });
});
