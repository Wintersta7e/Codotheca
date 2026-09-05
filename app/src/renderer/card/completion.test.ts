import { describe, expect, it } from 'vitest';
import { LADDER_RUNGS } from '../theme/tokens';
import { densityStep } from './geometry';
import {
  ARCHIVED_GLASS,
  EM_DASH,
  GOLD_NOTCH,
  frameToken,
  paintsLadderRung,
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
