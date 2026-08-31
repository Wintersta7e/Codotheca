import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SceneHash } from '../../generated/protocol';
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
  artSceneHash: 'aa' as SceneHash,
  artState: 'ready',
  ...over,
});

const draw = (over: Partial<HeroRow> = {}, heroSrc = ''): HTMLElement =>
  render(
    <HeroFrame row={row(over)} heroSrc={heroSrc} halo={{ shadow: null, opacity: 1 }} chips={[]}>
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

describe('§7.8a: nothing else in phase 1 carries a second copy of the pin', () => {
  it('mounts no pin control on the hero, though the geometry for one exists', () => {
    expect(draw().querySelector('.cdt-pin')).toBeNull();
    expect(bandsFor('hero').pin.box).toBe(14);
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
