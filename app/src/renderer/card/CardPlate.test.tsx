import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { CardPlate, type CardPlateProps } from './CardPlate';
import { ARCHIVED_GLASS } from './completion';
import { bandsFor } from './geometry';

afterEach(cleanup);

const numbersIn = (text: string): number[] =>
  [...text.matchAll(/-?\d*\.?\d+/g)].map((m) => Number(m[0]));

const draw = (over: Partial<CardPlateProps> = {}): HTMLElement =>
  render(
    <CardPlate surface="card" isArchived={false} art={null} {...over}>
      <span data-testid="bands" />
    </CardPlate>,
  ).container;

describe('the plate is the ground, and the bitmap sits above it', () => {
  it('renders no image at all until the caller has one — the CSS plate is a finished card', () => {
    expect(draw().querySelector('img')).toBeNull();
    expect(draw().querySelector('.cdt-plate')).not.toBeNull();
  });

  it('puts the bitmap under every band, so DOM order is the stack', () => {
    const container = draw({ art: <img alt="" data-testid="art" src="codotheca://art/aa/card" /> });
    const plate = container.querySelector('.cdt-plate');
    expect(plate?.firstElementChild?.getAttribute('data-testid')).toBe('art');
  });

  it('renders the bands last, so nothing spanning the plate paints over them', () => {
    const plate = draw().querySelector('.cdt-plate');
    expect(plate?.lastElementChild?.getAttribute('data-testid')).toBe('bands');
  });
});

describe('only layers carrying no content may span the plate (§7.7)', () => {
  it('renders the sheen, the specular sweep and the scan line, and nothing else spanning', () => {
    const container = draw();
    for (const cls of ['.cdt-sheen', '.cdt-specular', '.cdt-scanline']) {
      expect(container.querySelector(cls)).not.toBeNull();
    }
    for (const spanning of container.querySelectorAll('.cdt-sheen, .cdt-specular, .cdt-scanline')) {
      expect(spanning.getAttribute('aria-hidden')).toBe('true');
      expect(spanning.textContent).toBe('');
    }
  });

  it('takes the sheen alpha from the surface‘s own band table, never from the other one', () => {
    const card = draw().querySelector<HTMLElement>('.cdt-sheen');
    const hero = draw({ surface: 'hero' }).querySelector<HTMLElement>('.cdt-sheen');
    expect(card?.style.backgroundImage).toContain(bandsFor('card').sheenAlpha);
    expect(hero?.style.backgroundImage).toContain(bandsFor('hero').sheenAlpha);
    expect(card?.style.backgroundImage).not.toBe(hero?.style.backgroundImage);
  });

  // The raster carries no text — type is DOM — so the plate's own subtree is furniture, and the
  // only strings inside it are the caller's bands.
  it('contributes no text of its own', () => {
    const plate = draw().querySelector('.cdt-plate');
    expect(plate?.textContent).toBe('');
  });
});

describe('§7.7a: is_archived keeps only its sealed markers', () => {
  it('adds the glass overlay and nothing that touches completion', () => {
    expect(draw().querySelector('.cdt-glass')).toBeNull();
    expect(draw({ isArchived: true }).querySelector('.cdt-glass')).not.toBeNull();
  });

  it('paints the overlay from the one constant, so no second gradient can drift from it', () => {
    const glass = draw({ isArchived: true }).querySelector<HTMLElement>('.cdt-glass');
    expect(glass?.style.backgroundImage).not.toBe('');
    expect(glass?.getAttribute('aria-hidden')).toBe('true');
    // A gradient is its numbers, not their spelling, and two rewriters sit between the constant
    // and this assertion: a CSS formatter turns `/ .16` into `/ 0.16`, and jsdom echoes an
    // inline `rgb(255 255 255 / .16)` back as `rgba(255, 255, 255, .16)`. Matching either
    // spelling would fail against a value that is correct.
    expect(numbersIn(glass?.style.backgroundImage ?? '')).toEqual(numbersIn(ARCHIVED_GLASS));
  });
});

describe('§7.8‘s sigil layer', () => {
  it('mounts nothing in phase 1, because no section gives the watermark a glyph', () => {
    expect(draw().querySelector('.cdt-sigil')).toBeNull();
  });

  it('mounts one, aria-hidden, if a later phase ever supplies a glyph', () => {
    const container = draw({ sigil: <span data-testid="glyph" /> });
    const sigil = container.querySelector('.cdt-sigil');
    expect(sigil).not.toBeNull();
    expect(sigil?.getAttribute('aria-hidden')).toBe('true');
  });
});
