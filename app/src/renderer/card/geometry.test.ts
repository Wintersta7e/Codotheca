import { describe, expect, it } from 'vitest';
import { DOT_SIZE_PX } from '../derive/condition';
import { isOnScale } from '../theme/type';
import {
  DENSITY_STEPS,
  DOT_SURFACE,
  GRID_TILE_BANDS,
  HAZARD_TAPE_HEIGHT_PX,
  HERO_BANDS,
  bandsFor,
  densityStep,
} from './geometry';

describe('the grid tile table, verbatim from §7.7', () => {
  const t = GRID_TILE_BANDS;

  it('carries the tile chamfer, sheen and bezel', () => {
    expect(t.chamferToken).toBe('chamfer-card');
    expect(t.sheenAlpha).toBe('.09');
    expect(t.bezel).toBe(
      'inset 0 1px 0 0 rgb(255 255 255 / .13), inset 0 -22px 30px -22px rgb(0 0 0 / .85)',
    );
    expect(t.hairlineStops).toEqual(['18%', '82%']);
  });

  it('closes band 1 at 30px and opens band 3 at 31px', () => {
    expect(t.bands.band1Bottom).toBe(30);
    expect(t.bands.hairlineTop).toBe(30);
    expect(t.bands.band3Top).toBe(31);
    expect(t.bands.band3Bottom).toBe('34%');
    expect(t.bands.band4Top).toBe('34%');
  });

  it('puts the rank glyph in the right half only', () => {
    expect(t.rank.left).toBe('50%');
    expect(t.rank.width).toBe('50%');
    expect(t.rank.padding).toBe('0 10px');
    expect(t.rank.labelPx).toBe(7);
    expect(t.rank.labelMarginTop).toBe(2);
  });

  it('places band 4 — chips, vent, stripe, designation', () => {
    expect(t.chipColumn).toEqual({ left: 9, right: 9, gap: 3 });
    expect(t.vent).toEqual({ left: 9, right: 9, bottom: '38%', height: 14, opacity: 0.6 });
    expect(t.jewelStripe.bottom).toBe('34%');
    expect(t.jewelStripe.height).toBe(7);
    expect(t.jewelStripe.shadow).toBe(
      '0 -1px 0 0 rgb(255 255 255 / .14), 0 2px 6px -1px var(--cdt-jewel-30)',
    );
    expect(t.designation).toEqual({ fontPx: 7, tracking: '.18em', marginTop: 5 });
  });

  it('places band 1 furniture and band 5', () => {
    expect(t.languagePlate).toEqual({
      left: 9,
      top: 8,
      height: 16,
      padding: '0 6px',
      fontPx: 8.5,
      tracking: '.12em',
    });
    expect(t.dot).toEqual({ right: 10, top: 10 });
    expect(t.scrim).toEqual({ padding: '34px 12px 13px', stop: '42%' });
    expect(HAZARD_TAPE_HEIGHT_PX).toBe(5);
  });
});

describe('the hero table, which is a different table', () => {
  const h = HERO_BANDS;

  it('carries the hero chamfer, sheen and bezel', () => {
    expect(h.chamferToken).toBe('chamfer-hero');
    expect(h.sheenAlpha).toBe('.1');
    expect(h.bezel).toBe(
      'inset 0 1px 0 0 rgb(255 255 255 / .13), inset 0 -30px 40px -30px rgb(0 0 0 / .9)',
    );
    expect(h.hairlineStops).toEqual(['16%', '84%']);
  });

  it('closes band 1 at 36px and opens band 3 at 38px, not 37', () => {
    expect(h.bands.band1Bottom).toBe(36);
    expect(h.bands.hairlineTop).toBe(36);
    expect(h.bands.band3Top).toBe(38);
    expect(h.bands.band3Bottom).toBe('32%');
    expect(h.bands.band4Top).toBe('32%');
  });

  it('spans the full width in band 3 — the tile halves are not the hero halves', () => {
    expect(h.rank.left).toBe('0');
    expect(h.rank.width).toBe('100%');
    expect(h.rank.padding).toBe('0 13px');
    expect(h.rank.labelPx).toBe(7.5);
    expect(h.rank.labelMarginTop).toBe(3);
  });

  it('closes band 4 at bottom:31%, which is the edge conflation moves', () => {
    expect(h.jewelStripe.bottom).toBe('31%');
    expect(h.jewelStripe.height).toBe(8);
    expect(h.jewelStripe.shadow).toBe(
      '0 -1px 0 0 rgb(255 255 255 / .14), 0 2px 7px -1px var(--cdt-jewel-30)',
    );
    expect(h.chipColumn).toEqual({ left: 12, right: 12, gap: 4 });
    expect(h.vent).toEqual({ left: 12, right: 12, bottom: '35%', height: 17, opacity: 0.6 });
    expect(h.scrim).toEqual({ padding: '30px 13px 13px', stop: '44%' });
  });
});

describe('the two tables are never one table with a scale factor', () => {
  it('differs at every edge the design README would have shared', () => {
    const differing: readonly (keyof typeof GRID_TILE_BANDS.bands)[] = [
      'band1Bottom',
      'hairlineTop',
      'band3Top',
      'band3Bottom',
      'band4Top',
    ];
    for (const key of differing) {
      expect(GRID_TILE_BANDS.bands[key]).not.toBe(HERO_BANDS.bands[key]);
    }
    expect(GRID_TILE_BANDS.jewelStripe.bottom).not.toBe(HERO_BANDS.jewelStripe.bottom);
    expect(GRID_TILE_BANDS.vent.bottom).not.toBe(HERO_BANDS.vent.bottom);
    expect(GRID_TILE_BANDS.rank.left).not.toBe(HERO_BANDS.rank.left);
  });

  it('starts band 3 at neither table on the struck 30px', () => {
    // The design README's band-3 start is 30px. §7.7 strikes it as a value never built.
    expect(GRID_TILE_BANDS.bands.band3Top).not.toBe(30);
    expect(HERO_BANDS.bands.band3Top).not.toBe(30);
  });

  it('never lets band 1 and the hairline overlap', () => {
    for (const table of [GRID_TILE_BANDS, HERO_BANDS]) {
      expect(table.bands.hairlineTop).toBe(table.bands.band1Bottom);
      expect(table.bands.band3Top).toBeGreaterThan(table.bands.hairlineTop);
      expect(table.languagePlate.top + table.languagePlate.height).toBeLessThanOrEqual(
        table.bands.band1Bottom,
      );
    }
  });

  it('resolves a surface to its table', () => {
    expect(bandsFor('card')).toBe(GRID_TILE_BANDS);
    expect(bandsFor('hero')).toBe(HERO_BANDS);
  });
});

describe('the pin sits in band 1, inboard of the dot, below the tape', () => {
  it('reproduces §7.8a on the tile', () => {
    const p = GRID_TILE_BANDS.pin;
    expect({ box: p.box, right: p.right, top: p.top }).toEqual({ box: 12, right: 24, top: 9 });
    expect([p.barW, p.barH, p.shaftW, p.shaftH]).toEqual([9, 2, 2, 5]);
    // §7.8a states the tile's hit target outright; the derivation must reproduce it.
    expect({ hit: p.hit, hitRight: p.hitRight, hitTop: p.hitTop }).toEqual({
      hit: 24,
      hitRight: 18,
      hitTop: 3,
    });
    // The tile is the surface §7.8a measures against the tape: "4px below the tape".
    expect(p.top).toBe(HAZARD_TAPE_HEIGHT_PX + 4);
  });

  it('takes its own values on the hero, centred the same way', () => {
    const p = HERO_BANDS.pin;
    expect({ box: p.box, right: p.right, top: p.top }).toEqual({ box: 14, right: 26, top: 10 });
    expect([p.barW, p.barH, p.shaftW, p.shaftH]).toEqual([11, 2, 2, 6]);
    expect({ hit: p.hit, hitRight: p.hitRight, hitTop: p.hitTop }).toEqual({
      hit: 26,
      hitRight: 20,
      hitTop: 4,
    });
    // The hero clears the tape by 5px, not the tile's 4px. §7.8a states the clearance for the
    // tile alone and gives the hero its own `top`; one clearance asserted for both surfaces is
    // the conflation the two tables exist to prevent, a field further in.
    expect(p.top - HAZARD_TAPE_HEIGHT_PX).not.toBe(GRID_TILE_BANDS.pin.top - HAZARD_TAPE_HEIGHT_PX);
  });

  it('is inboard of the dot by 6px and clears the hazard tape on both surfaces', () => {
    for (const table of [GRID_TILE_BANDS, HERO_BANDS]) {
      const dotSize = DOT_SIZE_PX[DOT_SURFACE[table.surface]];
      expect(table.pin.right).toBe(table.dot.right + dotSize + 6);
      expect(table.pin.top).toBeGreaterThanOrEqual(HAZARD_TAPE_HEIGHT_PX + 4);
      expect(table.pin.top + table.pin.box).toBeLessThanOrEqual(table.bands.band1Bottom);
    }
  });

  it('abuts the condition dot box without overlapping it, and stays inside band 1', () => {
    // Criterion 55's own clause. The hit target is centred on the box, so `hitRight` is exactly
    // where the two boxes meet: one pixel less and the pin eats the dot's target.
    for (const table of [GRID_TILE_BANDS, HERO_BANDS]) {
      const dotSize = DOT_SIZE_PX[DOT_SURFACE[table.surface]];
      expect(table.pin.hitRight).toBe(table.dot.right + dotSize);
      expect(table.pin.hitRight).toBe(table.pin.right - (table.pin.hit - table.pin.box) / 2);
      expect(table.pin.hitTop).toBe(table.pin.top - (table.pin.hit - table.pin.box) / 2);
      expect(table.pin.hitTop + table.pin.hit).toBeLessThanOrEqual(table.bands.band1Bottom);
    }
  });

  it('maps each card surface to the dot surface §5.4a names', () => {
    expect(DOT_SURFACE).toEqual({ card: 'gridTile', hero: 'heroTile' });
    expect(DOT_SIZE_PX[DOT_SURFACE.card]).toBe(8);
    expect(DOT_SIZE_PX[DOT_SURFACE.hero]).toBe(9);
  });
});

describe('density is three steps, not a scale factor', () => {
  it('drops the strip and the identity line below 156px', () => {
    const step = densityStep(140);
    expect(step).toEqual({
      name: 'compact',
      namePx: 15,
      glyphPx: 30,
      showDescription: false,
      showIdentity: false,
      showStrip: false,
    });
  });

  it('adds the identity line and the strip at the 186px default', () => {
    expect(densityStep(186)).toEqual({
      name: 'default',
      namePx: 17,
      glyphPx: 34,
      showDescription: false,
      showIdentity: true,
      showStrip: true,
    });
    expect(densityStep(156)).toBe(densityStep(186));
    expect(densityStep(205)).toBe(densityStep(186));
  });

  it('adds the description at 206px and up', () => {
    expect(densityStep(206)).toEqual({
      name: 'roomy',
      namePx: 19,
      glyphPx: 38,
      showDescription: true,
      showIdentity: true,
      showStrip: true,
    });
    expect(densityStep(400)).toBe(densityStep(206));
  });

  it('emits only sizes the §8.7 scale carries', () => {
    for (const step of DENSITY_STEPS) {
      expect(isOnScale('display', step.namePx)).toBe(true);
      expect(isOnScale('display', step.glyphPx)).toBe(true);
    }
    expect(isOnScale('mono', GRID_TILE_BANDS.rank.labelPx)).toBe(true);
    expect(isOnScale('mono', HERO_BANDS.rank.labelPx)).toBe(true);
    expect(isOnScale('mono', GRID_TILE_BANDS.languagePlate.fontPx)).toBe(true);
    expect(isOnScale('mono', HERO_BANDS.languagePlate.fontPx)).toBe(true);
  });
});
