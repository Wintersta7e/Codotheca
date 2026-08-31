import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { appearanceFor } from '../art/appearance';
import { LADDER_RUNGS } from '../theme/tokens';
import { Card, type CardProps } from './Card';
import { uncomputedRank } from './completion';
import { HAZARD_TAPE_HEIGHT_PX, bandsFor } from './geometry';

afterEach(cleanup);

const look = appearanceFor({ seedBasename: 'atlas', rerollOffset: 0 }, 0, 'Rust');
const rank = uncomputedRank('gridCard', { completionLit: null, isReference: false, density: 186 });

const props = (over: Partial<CardProps> = {}): CardProps => ({
  surface: 'card',
  appearance: look,
  frameToken: 'unknown',
  density: 186,
  isArchived: false,
  isReference: false,
  art: null,
  bands: {
    languageCode: 'RS',
    designation: 'RS-42 / MK-III',
    hazard: false,
    conditionSignal: 'dormant',
    rank,
    chips: [],
    pin: null,
  },
  halo: { shadow: '0 0 12px -6px var(--sig)', opacity: 1 },
  hovered: false,
  focused: false,
  selected: false,
  children: <span data-testid="band5" />,
  ...over,
});

const draw = (over: Partial<CardProps> = {}): HTMLElement =>
  render(<Card {...props(over)} />).container;
const px = (v: string): number => Number.parseFloat(v);
const tight = (v: string): string => v.replace(/\s+/g, '');

/** `#333c45` → `rgb(51, 60, 69)`, which is the only form an inline colour reaches the DOM in. */
const asRgb = (hex: string): string => {
  const n = Number.parseInt(hex.slice(1), 16);
  return `rgb(${String((n >> 16) & 255)}, ${String((n >> 8) & 255)}, ${String(n & 255)})`;
};

describe('clip-path deletes what sits outside it, so three things are unclipped siblings', () => {
  it('puts the halo, the bloom and the frame gap outside the clipped card', () => {
    const container = draw();
    const frame = container.querySelector('.cdt-card-frame');
    for (const cls of ['.cdt-card-halo', '.cdt-bloom', '.cdt-frame-gap']) {
      const node = container.querySelector(cls);
      expect(node, `${cls} is not mounted`).not.toBeNull();
      expect(node?.parentElement).toBe(frame);
      expect(node?.closest('.cdt-card')).toBeNull();
    }
  });

  it('mounts the gap at the negative offset the clip would have eaten', () => {
    const gap = draw().querySelector<HTMLElement>('.cdt-frame-gap');
    expect(gap?.style.top).toBe('-1px');
    expect(gap?.style.left).toBe('39%');
    expect(gap?.style.width).toBe('22%');
  });

  it('fills the gap with the surface behind the frame, as a token and never a hex', () => {
    const tile = draw().querySelector<HTMLElement>('.cdt-frame-gap');
    expect(tile?.style.getPropertyValue('background-color')).toBe('var(--surface-1)');
    const heroRank = uncomputedRank('hero', {
      completionLit: null,
      isReference: false,
      density: 186,
    });
    const hero = draw({ surface: 'hero', bands: { ...props().bands, rank: heroRank } });
    expect(
      hero.querySelector<HTMLElement>('.cdt-frame-gap')?.style.getPropertyValue('background-color'),
    ).toBe('var(--surface-0)');
  });
});

describe('the custom-property bag reaches the siblings, not only the card', () => {
  it('sets every value on the frame, because the bloom is the card‘s sibling', () => {
    const frame = draw().querySelector<HTMLElement>('.cdt-card-frame');
    expect(frame?.style.getPropertyValue('--cdt-jewel')).toBe(look.jewel);
    expect(frame?.style.getPropertyValue('--cdt-bloom')).not.toBe('');
    expect(frame?.style.getPropertyValue('--cdt-frame')).toBe('var(--unknown)');
    // The card itself carries none of them: a bag set there would never reach the bloom.
    const card = draw().querySelector<HTMLElement>('.cdt-card');
    expect(card?.style.getPropertyValue('--cdt-bloom')).toBe('');
  });

  it('carries the dip as a property so the tier clamp still outranks it', () => {
    const halo = draw({ halo: { shadow: 'x', opacity: 0.8 } }).querySelector<HTMLElement>(
      '.cdt-card-halo',
    );
    expect(halo?.style.opacity).toBe('');
    const frame = draw({ halo: { shadow: 'x', opacity: 0.8 } }).querySelector<HTMLElement>(
      '.cdt-card-frame',
    );
    expect(frame?.style.getPropertyValue('--cdt-halo-opacity')).toBe('0.8');
  });

  it('marks hover on the frame and on the card, because card.css selects through both', () => {
    const container = draw({ hovered: true });
    expect(container.querySelector('.cdt-card-frame')?.getAttribute('data-hovered')).toBe('true');
    expect(container.querySelector('.cdt-card')?.getAttribute('data-hovered')).toBe('true');
  });
});

describe('§7.7: no element may occupy two bands', () => {
  const withPin = (over: Partial<CardProps> = {}): HTMLElement =>
    draw({
      bands: {
        ...props().bands,
        hazard: true,
        pin: {
          projectName: 'atlas',
          isPinned: false,
          surface: 'card',
          visible: true,
          onToggle: () => {},
        },
      },
      ...over,
    });

  it('keeps every band-1 occupant inside band 1', () => {
    const table = bandsFor('card');
    const container = withPin();
    for (const cls of ['.cdt-langplate', '.cdt-dot', '.cdt-pin']) {
      const node = container.querySelector<HTMLElement>(cls);
      expect(node, `${cls} is not mounted`).not.toBeNull();
      expect(px(node?.style.top ?? '')).toBeGreaterThanOrEqual(0);
      expect(px(node?.style.top ?? '')).toBeLessThan(table.bands.band1Bottom);
    }
  });

  it('starts the pin MARK below the one band-1 element that spans the plate', () => {
    // §7.8a's clearance is the mark's, not the hit target's: the button is a 24px box centred
    // on a 12px silhouette, so its own `top` is deliberately above the tape and the assertion
    // that matters is where the drawn mark starts.
    const container = withPin();
    const tape = container.querySelector<HTMLElement>('.cdt-hazard');
    expect(px(tape?.style.height ?? '')).toBe(HAZARD_TAPE_HEIGHT_PX);
    const button = container.querySelector<HTMLElement>('.cdt-pin');
    const mark = container.querySelector<HTMLElement>('.cdt-pin-silhouette');
    const markTop = px(button?.style.top ?? '') + px(mark?.style.top ?? '');
    expect(markTop).toBe(bandsFor('card').pin.top);
    expect(markTop).toBeGreaterThanOrEqual(HAZARD_TAPE_HEIGHT_PX + 4);
  });

  it('opens band 3 below the hairline and closes it above band 4', () => {
    const table = bandsFor('card');
    const container = draw();
    expect(px(container.querySelector<HTMLElement>('.cdt-hairline')?.style.top ?? '')).toBe(
      table.bands.hairlineTop,
    );
    expect(px(container.querySelector<HTMLElement>('.cdt-rank')?.style.top ?? '')).toBe(
      table.bands.band3Top,
    );
    // jsdom's CSS parser folds `calc(100% - 34%)` to `calc(66%)`; a browser keeps the
    // expression. The assertion is the arithmetic either way, and it is derived from the table
    // rather than retyped, so a changed band edge moves it.
    const closesAt = 100 - Number.parseFloat(table.bands.band3Bottom);
    expect(tight(container.querySelector<HTMLElement>('.cdt-rank')?.style.bottom ?? '')).toBe(
      `calc(${String(closesAt)}%)`,
    );
    // Band 4 opens exactly where band 3 closed. The two are one edge, stated twice.
    expect(container.querySelector<HTMLElement>('.cdt-chips')?.style.top).toBe(
      table.bands.band4Top,
    );
    expect(table.bands.band4Top).toBe(table.bands.band3Bottom);
  });

  it('reads the hero‘s edges from the hero table and never from the tile‘s', () => {
    const heroRank = uncomputedRank('hero', {
      completionLit: null,
      isReference: false,
      density: 186,
    });
    const container = draw({ surface: 'hero', bands: { ...props().bands, rank: heroRank } });
    expect(px(container.querySelector<HTMLElement>('.cdt-hairline')?.style.top ?? '')).toBe(
      bandsFor('hero').bands.hairlineTop,
    );
    expect(container.querySelector<HTMLElement>('.cdt-stripe')?.style.bottom).toBe('31%');
    expect(container.querySelector<HTMLElement>('.cdt-stripe')?.style.bottom).not.toBe(
      bandsFor('card').jewelStripe.bottom,
    );
  });

  it('drops the whole band-4 strip below the density that carries it', () => {
    const compact = draw({ density: 140 });
    expect(compact.querySelector('.cdt-chips')).toBeNull();
    expect(compact.querySelector('.cdt-vent')).toBeNull();
    expect(compact.querySelector('.cdt-stripe')).toBeNull();
  });
});

describe('§7.7a and §11.7: the absence is stated, and it is stated to the tree too', () => {
  it('draws the em dash, the label and the accessible name, and never a ladder colour', () => {
    const container = draw();
    expect(container.querySelector('.cdt-rank-glyph')?.textContent).toBe('—');
    expect(container.querySelector('.cdt-rank-label')?.textContent).toBe('NOT COMPUTED');
    expect(screen.getByText('Completion not computed')).toBeTruthy();
    // Both spellings: an inline `#333c45` reaches the DOM as `rgb(51, 60, 69)` and a hex-only
    // guard passes against a card that really is painting a rung.
    for (const rung of LADDER_RUNGS) {
      expect(container.innerHTML, rung).not.toContain(rung);
      expect(container.innerHTML, `${rung} as rgb()`).not.toContain(asRgb(rung));
    }
    expect(container.innerHTML).not.toContain('0/10');
  });

  it('renders no tick row anywhere — no row is "not computed", ten dark ticks is "0 of 10"', () => {
    expect(draw().querySelectorAll('[data-tick]')).toHaveLength(0);
  });

  it('puts no digit at all in the rank band', () => {
    expect(draw().querySelector('.cdt-rank')?.textContent).not.toMatch(/\d/);
  });

  it('takes the reference frame when the row is one, and keeps the gap', () => {
    const container = draw({ frameToken: 'tier-ref', isReference: true });
    expect(
      container
        .querySelector<HTMLElement>('.cdt-card-frame')
        ?.style.getPropertyValue('--cdt-frame'),
    ).toBe('var(--tier-ref)');
    expect(container.querySelector('.cdt-frame-gap')).not.toBeNull();
  });
});

describe('the language plate renders only when there is a language', () => {
  it('omits the whole plate on a NULL primary_language', () => {
    expect(
      draw({ bands: { ...props().bands, languageCode: null } }).querySelector('.cdt-langplate'),
    ).toBeNull();
  });
});

describe('the pointer routes mirror the key table', () => {
  it('plays on a click and opens the page on Shift+click', () => {
    const onClick = vi.fn();
    const container = draw({ onClick });
    const card = container.querySelector('.cdt-card');
    if (card === null) throw new Error('no card');
    fireEvent.click(card);
    fireEvent.click(card, { shiftKey: true });
    expect(onClick).toHaveBeenNthCalledWith(1, false);
    expect(onClick).toHaveBeenNthCalledWith(2, true);
  });

  it('reports hover through the caller‘s handlers, never through CSS :hover', () => {
    const onMouseEnter = vi.fn();
    const onMouseLeave = vi.fn();
    const container = draw({ onMouseEnter, onMouseLeave });
    const card = container.querySelector('.cdt-card');
    if (card === null) throw new Error('no card');
    fireEvent.mouseEnter(card);
    fireEvent.mouseLeave(card);
    expect(onMouseEnter).toHaveBeenCalledOnce();
    expect(onMouseLeave).toHaveBeenCalledOnce();
  });
});

describe('§11.7: the cell states its role and its selection, and names nothing itself', () => {
  it('carries the caller‘s role and roving tabindex and no aria-label', () => {
    const card = draw({ role: 'gridcell', tabIndex: 0, selected: true }).querySelector('.cdt-card');
    expect(card?.getAttribute('role')).toBe('gridcell');
    expect(card?.getAttribute('tabindex')).toBe('0');
    expect(card?.getAttribute('aria-selected')).toBe('true');
    expect(card?.hasAttribute('aria-label')).toBe(false);
  });

  it('sets no aria-selected at all when the cell is not selected', () => {
    expect(draw().querySelector('.cdt-card')?.hasAttribute('aria-selected')).toBe(false);
  });
});
