import { describe, expect, it } from 'vitest';
import { appearanceFor } from '../art/appearance';
import { LIGHTEST_PLATE_STOP, NON_TEXT_FLOOR, ratioOf } from '../theme/contrast';
import {
  BRACKET_STAGGER_S,
  DOT_HOVER_TRANSFORM,
  HOVER_EFFECTS,
  LIFT_TRANSFORM,
  PRESS_TRANSFORM,
  bloomShadow,
  cardCustomProperties,
  effectsAt,
  ledGradient,
  scanGradient,
  selectionRing,
} from './interaction';

const look = appearanceFor({ seedBasename: 'aurora', rerollOffset: 0 }, 0, 'Rust');

describe('the ten effects §7.8 tabulates', () => {
  it('names all ten, once each', () => {
    expect(HOVER_EFFECTS).toHaveLength(10);
    expect(HOVER_EFFECTS.map((e) => e.id)).toEqual([
      'lift',
      'frame',
      'ledRun',
      'bloom',
      'specular',
      'brackets',
      'scanLine',
      'sigil',
      'conditionDot',
      'dataStrip',
    ]);
  });

  it('puts the bloom on the unclipped sibling, never on the clipped card', () => {
    expect(HOVER_EFFECTS.find((e) => e.id === 'bloom')?.element).toBe('.cdt-bloom');
    expect(HOVER_EFFECTS.find((e) => e.id === 'specular')?.element).toBe('.cdt-plate');
    expect(HOVER_EFFECTS.find((e) => e.id === 'ledRun')?.element).toBe('.cdt-card');
  });

  it('carries the two card transforms and the dot scale verbatim', () => {
    expect(LIFT_TRANSFORM).toBe('translateY(-7px) scale(1.025)');
    expect(PRESS_TRANSFORM).toBe('translateY(-3px) scale(.986)');
    expect(DOT_HOVER_TRANSFORM).toBe('scale(1.5)');
    expect(BRACKET_STAGGER_S).toEqual(['0s', '.03s', '.06s', '.09s']);
  });
});

describe('the tier gates transitions and travelling highlights, never states', () => {
  it('runs all ten at full', () => {
    expect(effectsAt('full')).toHaveLength(10);
  });

  it('keeps frame, bloom and the data strip at reduced and drops the four travellers', () => {
    const reduced = effectsAt('reduced');
    expect(reduced).toContain('frame');
    expect(reduced).toContain('bloom');
    expect(reduced).toContain('dataStrip');
    for (const dropped of ['ledRun', 'specular', 'brackets', 'scanLine']) {
      expect(reduced).not.toContain(dropped);
    }
  });

  it('drops every transform at reduced — the lift and the dot scale are transforms', () => {
    expect(effectsAt('reduced')).not.toContain('lift');
    expect(effectsAt('reduced')).not.toContain('conditionDot');
  });

  it('renders the same set at off, instantly — a hovered card still takes frame and bloom', () => {
    expect(effectsAt('off')).toEqual(effectsAt('reduced'));
  });
});

describe('the selection ring is the focus ring', () => {
  it('is two inset segments and nothing else', () => {
    const ring = selectionRing(look);
    const segments = ring.split(/,(?![^(]*\))/).map((s) => s.trim());
    expect(segments).toHaveLength(2);
    for (const segment of segments) expect(segment.startsWith('inset ')).toBe(true);
  });

  it('draws the hard ring in jewelInk and keeps the faded jewel only for the soft bloom', () => {
    const ring = selectionRing(look);
    expect(ring).toContain(`inset 0 0 0 2px ${look.jewelInk}`);
    expect(ring).toContain('inset 0 0 22px -6px oklch(0.53 0.135 27 / .55)');
    expect(ring.startsWith(`inset 0 0 0 2px ${look.jewelInk}`)).toBe(true);
  });

  it('clears the non-text floor on every jewel, where the jewel itself does not', () => {
    // Measured, not quoted. §7.8's prose puts a jewel ring at 1.84–2.05 : 1 against the lightest
    // tone a plate emits; the real range over the eight hue bins and both fades is 2.65–4.79,
    // which is worse than a flat failure — the ring would be legible on most cards and invisible
    // on some, and nothing on screen would say which. `jewelInk` is 11.1–11.7 on every one.
    const stop = LIGHTEST_PLATE_STOP;
    const lightestPlate = `oklch(${String(stop.l)} ${String(stop.c)} ${String(stop.hue)})`;

    let jewelsBelowFloor = 0;
    let worstInk = Number.POSITIVE_INFINITY;
    for (const seedBasename of ['aurora', 'a', 'b', 'c', 'zz', 'one', 'two', 'three', 'seven']) {
      for (const fade of [0, 0.25]) {
        const a = appearanceFor({ seedBasename, rerollOffset: 0 }, fade, null);
        if (ratioOf(a.jewel, lightestPlate) < NON_TEXT_FLOOR) jewelsBelowFloor += 1;
        worstInk = Math.min(worstInk, ratioOf(a.jewelInk, lightestPlate));
      }
    }
    expect(worstInk).toBeGreaterThan(NON_TEXT_FLOOR);
    expect(worstInk).toBeGreaterThan(10);
    expect(jewelsBelowFloor).toBeGreaterThan(0);

    // The same card, faded because it is a reference or archived, is one of them — so this is
    // not an exotic hue, it is the ordinary card in its ordinary second state.
    const faded = appearanceFor({ seedBasename: 'aurora', rerollOffset: 0 }, 0.25, null);
    expect(ratioOf(faded.jewel, lightestPlate)).toBeLessThan(NON_TEXT_FLOOR);
    expect(ratioOf(faded.jewelInk, lightestPlate)).toBe(ratioOf(look.jewelInk, lightestPlate));
  });
});

describe('the bloom is an outer shadow and lives off the card', () => {
  it('carries the two published segments and no inset', () => {
    const bloom = bloomShadow(look);
    expect(bloom).toBe(
      '0 22px 34px -16px rgb(0 0 0 / .8), 0 0 32px -8px oklch(0.53 0.135 27 / .85)',
    );
    expect(bloom).not.toContain('inset');
  });
});

describe('all hover light is that card own jewel', () => {
  it('never emits the app accent, which the prototype leaked into one layer', () => {
    const surfaces = [
      ledGradient(look),
      scanGradient(look),
      bloomShadow(look),
      selectionRing(look),
    ].join(' ');
    expect(surfaces).not.toContain('#4a9dff');
    expect(surfaces).not.toContain('rgb(74 157 255');
    expect(surfaces).not.toContain('var(--sig)');
  });

  it('emits the LED run and the scan line at their published stops', () => {
    expect(ledGradient(look)).toBe(
      'linear-gradient(90deg, transparent 0 6%, oklch(0.53 0.135 27) 16%, #ffffff 23%, ' +
        'oklch(0.53 0.135 27) 30%, transparent 42%)',
    );
    expect(scanGradient(look)).toBe(
      'linear-gradient(180deg, transparent, rgb(255 255 255 / .18) 68%, ' +
        'oklch(0.53 0.135 27 / .8) 96%, transparent)',
    );
  });
});

describe('the custom-property bag is how the jewel reaches the stylesheet', () => {
  it('namespaces every property so plan 12 gate accepts it', () => {
    const bag = cardCustomProperties(look, 'var(--unknown)');
    for (const key of Object.keys(bag)) expect(key.startsWith('--cdt-')).toBe(true);
  });

  it('carries every value card.css reads, and no bare property name', () => {
    const bag = cardCustomProperties(look, 'var(--unknown)');
    expect(Object.keys(bag).sort()).toEqual([
      '--cdt-frame',
      '--cdt-greebling',
      '--cdt-jewel',
      '--cdt-jewel-12',
      '--cdt-jewel-30',
      '--cdt-jewel-55',
      '--cdt-jewel-80',
      '--cdt-jewel-85',
      '--cdt-jewel-ink',
      '--cdt-led',
      '--cdt-plate',
      '--cdt-scan',
    ]);
    expect(bag['--cdt-frame']).toBe('var(--unknown)');
    expect(bag['--cdt-jewel-ink']).toBe(look.jewelInk);
    expect(bag['--cdt-plate']).toBe(look.plate);
  });

  it('sets a value, never a property — a background-image set inline would beat the tier gate', () => {
    const bag = cardCustomProperties(look, 'var(--unknown)');
    for (const key of Object.keys(bag)) {
      expect(key).not.toBe('backgroundImage');
      expect(key).not.toBe('background-image');
    }
  });
});
