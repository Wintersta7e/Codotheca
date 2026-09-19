import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { LocationRef, SceneHash } from '../../generated/protocol';
import { HeroFrame, type HeroRow } from './HeroFrame';
import { bandsFor } from './geometry';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const row = (over: Partial<HeroRow> = {}): HeroRow => ({
  seedBasename: 'atlas',
  rerollOffset: 0,
  primaryLanguage: 'Rust',
  isReference: false,
  isArchived: false,
  conditionSignal: 'dormant',
  completionLit: null,
  // [p3] Both NULL together: the pair is a projection and §1.10's CHECK pairs them.
  completionApplicable: null,
  artSceneHash: 'aa' as SceneHash,
  artState: 'ready',
  // A located project. §23.5's blueprint hero is the zero-location case and asks for it by name.
  primaryLocation: { id: 10 as LocationRef['id'], pathDisplay: '/w/atlas' },
  ...over,
});

const draw = (over: Partial<HeroRow> = {}, heroSrc = ''): HTMLElement =>
  render(
    <HeroFrame
      row={row(over)}
      heroSrc={heroSrc}
      halo={{ shadow: null, opacity: 1 }}
      chips={[]}
      pin={null}
    >
      <span data-testid="band5" />
    </HeroFrame>,
  ).container;

const drawWithPin = (): HTMLElement =>
  render(
    <HeroFrame
      row={row()}
      heroSrc=""
      halo={{ shadow: null, opacity: 1 }}
      chips={[]}
      pin={{
        projectName: 'atlas',
        isPinned: true,
        surface: 'hero',
        visible: true,
        tabIndex: 0,
        onToggle: vi.fn(),
      }}
    >
      <span data-testid="band5" />
    </HeroFrame>,
  ).container;

describe('the hero is not the tile at hero scale', () => {
  it('takes every edge from the second table', () => {
    const container = draw();
    expect(container.querySelector('.cdt-card')?.getAttribute('data-surface')).toBe('hero');
    expect(container.querySelector<HTMLElement>('.cdt-stripe')?.style.bottom).toBe(
      bandsFor('hero').jewelStripe.bottom,
    );
    expect(container.querySelector<HTMLElement>('.cdt-stripe')?.style.bottom).not.toBe(
      bandsFor('card').jewelStripe.bottom,
    );
  });

  it('spans the rank band the full width, because the hero row states no halves', () => {
    const rank = draw().querySelector<HTMLElement>('.cdt-rank');
    // The table writes a bare `0`; jsdom serialises it back as `0px`. The edge is the number.
    expect(Number.parseFloat(rank?.style.left ?? '')).toBe(
      Number.parseFloat(bandsFor('hero').rank.left),
    );
    expect(rank?.style.width).toBe(bandsFor('hero').rank.width);
    expect(rank?.style.width).not.toBe(bandsFor('card').rank.width);
  });

  it('states the absence in the hero‘s own words', () => {
    expect(draw().querySelector('.cdt-rank-label')?.textContent).toBe('RANK NOT COMPUTED');
    expect(screen.getByText('Completion not computed')).toBeTruthy();
  });

  it('takes the reference frame when the project is one', () => {
    const container = draw({ isReference: true });
    expect(
      container
        .querySelector<HTMLElement>('.cdt-card-frame')
        ?.style.getPropertyValue('--cdt-frame'),
    ).toBe('var(--tier-ref)');
  });
});

describe('§7.8a: band 1 owns the pin, on both tile and hero', () => {
  it('draws the mark the caller hands it, at the hero row of the table', () => {
    // §7.8a gives the hero its own box, ground, glyph, rotation, ink and hit-target size, every
    // one different from the tile's. Its closing sentence rules out a *second copy* — a pin in
    // the page's chrome on top of this one — not this one.
    const button = drawWithPin().querySelector<HTMLElement>('.cdt-pin');
    expect(button).not.toBeNull();
    expect(button?.style.width).toBe(`${String(bandsFor('hero').pin.hit)}px`);
    expect(bandsFor('hero').pin.box).toBe(14);
    expect(bandsFor('hero').pin.hit).not.toBe(bandsFor('card').pin.hit);
  });

  it('mounts none when the caller supplies none, rather than inventing one', () => {
    expect(draw().querySelector('.cdt-pin')).toBeNull();
  });
});

describe('band 5 is a slot', () => {
  it('renders the caller‘s node inside the scrim and nothing of its own', () => {
    const scrim = draw().querySelector('.cdt-scrim');
    expect(scrim?.querySelector('[data-testid="band5"]')).not.toBeNull();
    expect(scrim?.textContent).toBe('');
  });

  it('mounts no status chip either — §7.7 gives the hero a column and §8.5 fills it', () => {
    expect(draw().querySelector('.cdt-chip')).toBeNull();
  });
});

describe('the address arrives from art.url, because that request is the demand', () => {
  it('keeps the plate when art.url answered with no address', () => {
    expect(draw({}, '').querySelector('img')).toBeNull();
  });

  it('paints the caller‘s address once it has decoded, and never composes its own', async () => {
    const seen: string[] = [];
    class FakeImage {
      public onload: (() => void) | null = null;
      public onerror: (() => void) | null = null;
      public set src(next: string) {
        seen.push(next);
        this.onload?.();
      }
    }
    vi.stubGlobal('Image', FakeImage);

    const container = draw({}, 'codotheca://art/deadbeef/hero');
    await act(async () => {
      await Promise.resolve();
    });
    expect(seen).toEqual(['codotheca://art/deadbeef/hero']);
    const art = container.querySelector('img');
    expect(art?.getAttribute('src')).toBe('codotheca://art/deadbeef/hero');
    // The row's own `artSceneHash` is `aa`; composing from it here would address the wrong file.
    expect(art?.getAttribute('src')).not.toContain('/aa/');
    expect(art?.getAttribute('alt')).toBe('');
  });
});
