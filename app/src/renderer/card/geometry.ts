import type { DotSurface } from '../derive/condition';

/**
 * §7.7's **two** reserved-band tables. They are different tables and nothing here derives one
 * from the other: the design prose calls the hero "the same object as the shelf tile at hero
 * scale, same reserved bands", and building that sentence literally moves the jewel stripe from
 * `bottom:31%` to `bottom:34%` — out of the band it closes. §7.7 supersedes the sentence and
 * these two constants are the whole of the allocation.
 *
 * Four separate overlap bugs came from ignoring these bands. Read them; do not remember them.
 */
export type CardSurface = 'card' | 'hero';

export interface BandExtents {
  readonly band1Bottom: number;
  readonly hairlineTop: number;
  readonly band3Top: number;
  readonly band3Bottom: string;
  readonly band4Top: string;
}

export interface LanguagePlateGeometry {
  readonly left: number;
  readonly top: number;
  readonly height: number;
  readonly padding: string;
  readonly fontPx: number;
  readonly tracking: string;
}

/** §5.4a owns the dot's size *and* its position; the size is imported, the inset is here. */
export interface DotInset {
  readonly right: number;
  readonly top: number;
}

export interface PinGeometry {
  readonly box: number;
  readonly right: number;
  readonly top: number;
  readonly barW: number;
  readonly barH: number;
  readonly shaftW: number;
  readonly shaftH: number;
  readonly hit: number;
  readonly hitRight: number;
  readonly hitTop: number;
}

export interface RankBandGeometry {
  readonly left: string;
  readonly width: string;
  readonly padding: string;
  readonly labelPx: number;
  readonly labelTracking: string;
  readonly labelMarginTop: number;
}

export interface CardBandTable {
  readonly surface: CardSurface;
  readonly chamferToken: 'chamfer-card' | 'chamfer-hero';
  readonly sheenAlpha: string;
  readonly bezel: string;
  readonly hairlineStops: readonly [string, string];
  readonly bands: BandExtents;
  readonly languagePlate: LanguagePlateGeometry;
  readonly dot: DotInset;
  readonly pin: PinGeometry;
  readonly rank: RankBandGeometry;
  readonly chipColumn: { readonly left: number; readonly right: number; readonly gap: number };
  readonly vent: {
    readonly left: number;
    readonly right: number;
    readonly bottom: string;
    readonly height: number;
    readonly opacity: number;
  };
  readonly jewelStripe: {
    readonly bottom: string;
    readonly height: number;
    readonly shadow: string;
  };
  readonly designation: {
    readonly fontPx: number;
    readonly tracking: string;
    readonly marginTop: number;
  };
  readonly scrim: { readonly padding: string; readonly stop: string };
}

/** Band 1's one plate-spanning element, and the one thing the pin has to clear (§7.8a). */
export const HAZARD_TAPE_HEIGHT_PX = 5;

export const DOT_SURFACE: Readonly<Record<CardSurface, DotSurface>> = {
  card: 'gridTile',
  hero: 'heroTile',
};

export const GRID_TILE_BANDS: CardBandTable = {
  surface: 'card',
  chamferToken: 'chamfer-card',
  sheenAlpha: '.09',
  bezel: 'inset 0 1px 0 0 rgb(255 255 255 / .13), inset 0 -22px 30px -22px rgb(0 0 0 / .85)',
  hairlineStops: ['18%', '82%'],
  bands: { band1Bottom: 30, hairlineTop: 30, band3Top: 31, band3Bottom: '34%', band4Top: '34%' },
  languagePlate: { left: 9, top: 8, height: 16, padding: '0 6px', fontPx: 8.5, tracking: '.12em' },
  dot: { right: 10, top: 10 },
  pin: {
    box: 12,
    right: 24,
    top: 9,
    barW: 9,
    barH: 2,
    shaftW: 2,
    shaftH: 5,
    hit: 24,
    hitRight: 18,
    hitTop: 3,
  },
  rank: {
    left: '50%',
    width: '50%',
    padding: '0 10px',
    labelPx: 7,
    labelTracking: '.24em',
    labelMarginTop: 2,
  },
  chipColumn: { left: 9, right: 9, gap: 3 },
  vent: { left: 9, right: 9, bottom: '38%', height: 14, opacity: 0.6 },
  jewelStripe: {
    bottom: '34%',
    height: 7,
    shadow: '0 -1px 0 0 rgb(255 255 255 / .14), 0 2px 6px -1px var(--cdt-jewel-30)',
  },
  designation: { fontPx: 7, tracking: '.18em', marginTop: 5 },
  scrim: { padding: '34px 12px 13px', stop: '42%' },
};

export const HERO_BANDS: CardBandTable = {
  surface: 'hero',
  chamferToken: 'chamfer-hero',
  sheenAlpha: '.1',
  bezel: 'inset 0 1px 0 0 rgb(255 255 255 / .13), inset 0 -30px 40px -30px rgb(0 0 0 / .9)',
  hairlineStops: ['16%', '84%'],
  bands: { band1Bottom: 36, hairlineTop: 36, band3Top: 38, band3Bottom: '32%', band4Top: '32%' },
  languagePlate: {
    left: 12,
    top: 11,
    height: 18,
    padding: '0 7px',
    fontPx: 9.5,
    tracking: '.12em',
  },
  dot: { right: 11, top: 11 },
  pin: {
    box: 14,
    right: 26,
    top: 10,
    barW: 11,
    barH: 2,
    shaftW: 2,
    shaftH: 6,
    hit: 26,
    hitRight: 20,
    hitTop: 4,
  },
  rank: {
    // §7.7's hero row gives padding and nothing else. "Right half only" is the tile's rule and
    // taking it here is the conflation the two tables exist to prevent.
    left: '0',
    width: '100%',
    padding: '0 13px',
    labelPx: 7.5,
    labelTracking: '.24em',
    labelMarginTop: 3,
  },
  chipColumn: { left: 12, right: 12, gap: 4 },
  vent: { left: 12, right: 12, bottom: '35%', height: 17, opacity: 0.6 },
  jewelStripe: {
    bottom: '31%',
    height: 8,
    shadow: '0 -1px 0 0 rgb(255 255 255 / .14), 0 2px 7px -1px var(--cdt-jewel-30)',
  },
  designation: { fontPx: 7, tracking: '.18em', marginTop: 5 },
  scrim: { padding: '30px 13px 13px', stop: '44%' },
};

export function bandsFor(surface: CardSurface): CardBandTable {
  return surface === 'hero' ? HERO_BANDS : GRID_TILE_BANDS;
}

export type DensityStepName = 'compact' | 'default' | 'roomy';

export interface DensityStep {
  readonly name: DensityStepName;
  readonly namePx: number;
  readonly glyphPx: number;
  readonly showDescription: boolean;
  readonly showIdentity: boolean;
  readonly showStrip: boolean;
}

/**
 * §7.7's density bands. The first-run scan tile is **not** a fourth step — §10.3a owns its
 * geometry outright and this allocation does not govern it at all.
 */
export const DENSITY_STEPS: readonly [DensityStep, DensityStep, DensityStep] = [
  {
    name: 'compact',
    namePx: 15,
    glyphPx: 30,
    showDescription: false,
    showIdentity: false,
    showStrip: false,
  },
  {
    name: 'default',
    namePx: 17,
    glyphPx: 34,
    showDescription: false,
    showIdentity: true,
    showStrip: true,
  },
  {
    name: 'roomy',
    namePx: 19,
    glyphPx: 38,
    showDescription: true,
    showIdentity: true,
    showStrip: true,
  },
];

export function densityStep(tilePx: number): DensityStep {
  if (tilePx < 156) return DENSITY_STEPS[0];
  if (tilePx < 206) return DENSITY_STEPS[1];
  return DENSITY_STEPS[2];
}
