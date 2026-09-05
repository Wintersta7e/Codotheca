import { act, cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectRow, SceneHash } from '../../generated/protocol';
import { renditionFor } from '../art/useCardBitmap';
import cardCss from '../styles/card.css?raw';
import motionCss from '../styles/motion.css?raw';
import { notClonedRow } from '../testing/projectRow';
import { LADDER_RUNGS, TOKENS } from '../theme/tokens';
import { ProjectCard, type ProjectCardProps } from './ProjectCard';
import { frameToken, paintsLadderRung } from './completion';

const HASH = '0123456789abcdef'.repeat(4) as SceneHash;

/** Every address a mounted card asked a detached `Image` to decode, in order. */
let requested: string[] = [];

class RecordingImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  #src = '';
  get src(): string {
    return this.#src;
  }
  set src(value: string) {
    this.#src = value;
    requested.push(value);
    // Resolve, so the card actually swaps to the bitmap and the `<img>` carries the address.
    queueMicrotask(() => this.onload?.());
  }
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  requested = [];
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
});

const row = (over: Partial<ProjectRow> = {}): ProjectRow =>
  notClonedRow({ artSceneHash: HASH, artState: 'ready', ...over });

const props = (over: Partial<ProjectCardProps> = {}): ProjectCardProps => ({
  row: row(),
  density: 186,
  rendition: 'card',
  selected: false,
  focused: false,
  now: 1_800_000_000,
  firstRunCompletedAt: null,
  session: null,
  haloOpacity: 1,
  onActivate: vi.fn(),
  onOpen: vi.fn(),
  onTogglePin: vi.fn(),
  onStopSession: vi.fn(),
  ...over,
});

const draw = (over: Partial<ProjectCardProps> = {}): HTMLElement =>
  render(<ProjectCard {...props(over)} />).container;

const frameOf = (container: HTMLElement): string =>
  container.querySelector<HTMLElement>('.cdt-card-frame')?.style.getPropertyValue('--cdt-frame') ??
  '';

describe('§23.5: the blueprint frame is --tier-blue, and it is not a rung', () => {
  it('resolves var(--tier-blue) on the real frame element', () => {
    expect(frameOf(draw())).toBe('var(--tier-blue)');
  });

  it('bites: a located row is unchanged and still takes the unknown frame', () => {
    const located = notClonedRow({
      artSceneHash: HASH,
      primaryLocation: { id: 10 as never, pathDisplay: '/w/row' },
      presence: 'present',
    });
    expect(frameOf(draw({ row: located }))).toBe('var(--unknown)');
  });

  it('is decided above the ladder, so it is not a member of it', () => {
    expect(paintsLadderRung(TOKENS['tier-blue'])).toBe(false);
    expect(paintsLadderRung('#2f4a5c')).toBe(false);
    // The counter-case, so the assertion is not vacuous: every rung IS one.
    expect(LADDER_RUNGS.length).toBeGreaterThan(0);
    for (const rung of LADDER_RUNGS) expect(paintsLadderRung(rung)).toBe(true);
  });

  it('draws no member of the completion ladder anywhere in the tile', () => {
    // The tile writes token **references** — `var(--tier-blue)` — and `paintsLadderRung` compares
    // hex, so a scan over the raw inline values could never fail. Every reference is resolved
    // through TOKENS first, and the resolved count is asserted: a run that resolved nothing is a
    // run that scanned nothing.
    const container = draw();
    const raw = [...container.querySelectorAll<HTMLElement>('*')].flatMap((el) => [
      el.style.getPropertyValue('--cdt-frame'),
      el.style.backgroundColor,
      el.style.color,
      el.style.borderColor,
    ]);
    const resolved: string[] = [];
    for (const value of raw) {
      const ref = /^var\(--([a-z0-9-]+)\)$/u.exec(value.trim());
      if (ref !== null) {
        const token = TOKENS[ref[1] as keyof typeof TOKENS] as string | undefined;
        if (token !== undefined) resolved.push(token);
        continue;
      }
      if (value !== '') resolved.push(value);
    }
    expect(resolved.length, 'the scan resolved no painted value').toBeGreaterThan(0);
    for (const value of resolved) {
      expect(paintsLadderRung(value), `${value} is a completion rung`).toBe(false);
    }
  });

  it('keeps §7.7a‘s uncomputed rank treatment, because completion is still NULL', () => {
    const container = draw();
    expect(container.querySelector('.cdt-rank-glyph')?.textContent ?? '').toContain('—');
    expect(container.querySelector('.cdt-rank-label')?.textContent).toBe('NOT COMPUTED');
    expect(container.querySelector('.cdt-frame-gap')).not.toBeNull();
  });

  it('reference still wins the frame, and the pair is what decides it', () => {
    expect(frameToken({ isReference: false, hasWorkingCopy: true })).toBe('unknown');
    expect(frameToken({ isReference: false, hasWorkingCopy: false })).toBe('tier-blue');
    expect(frameToken({ isReference: true, hasWorkingCopy: true })).toBe('tier-ref');
    expect(frameToken({ isReference: true, hasWorkingCopy: false })).toBe('tier-ref');
  });
});

describe('§23.5: the tile asks for the pass it needs, at its own address', () => {
  it('maps each surface to its own blueprint name', () => {
    expect(renditionFor('card', true)).toBe('card');
    expect(renditionFor('hero', true)).toBe('hero');
    expect(renditionFor('card', false)).toBe('card-blueprint');
    expect(renditionFor('hero', false)).toBe('hero-blueprint');
  });

  it('requests codotheca://art/<hash>/card-blueprint for a not-cloned tile', async () => {
    vi.stubGlobal('Image', RecordingImage);
    let container!: HTMLElement;
    // `act`, so the decode's microtask and the state update it causes both land before the
    // assertions: the point of this case is that the address reaches the DOM, not only the seam.
    await act(async () => {
      container = draw();
      await Promise.resolve();
    });
    expect(requested.length, 'the card decoded nothing').toBeGreaterThan(0);
    expect(requested).toContain(`codotheca://art/${HASH}/card-blueprint`);
    expect(requested).not.toContain(`codotheca://art/${HASH}/card`);
    // And the address the request produced is the one the DOM paints.
    const img = container.querySelector<HTMLImageElement>('img.cdt-art');
    expect(img?.getAttribute('src')).toBe(`codotheca://art/${HASH}/card-blueprint`);
  });

  it('requests the plain card for a located tile, so the swap has two addresses', async () => {
    vi.stubGlobal('Image', RecordingImage);
    await act(async () => {
      draw({
        row: notClonedRow({
          artSceneHash: HASH,
          primaryLocation: { id: 10 as never, pathDisplay: '/w/row' },
          presence: 'present',
        }),
      });
      await Promise.resolve();
    });
    expect(requested.length, 'the card decoded nothing').toBeGreaterThan(0);
    expect(requested).toContain(`codotheca://art/${HASH}/card`);
    expect(requested).not.toContain(`codotheca://art/${HASH}/card-blueprint`);
  });
});

/**
 * *"Only genuinely absent things (blueprint, reference) sit at zero."* `flicker.ts` already
 * returns false under a null `presence`, so this needs **no new rule** — it needs an assertion,
 * at every effects tier, resolved on a real element rather than matched in the stylesheet.
 */
describe('§11.6: the blueprint tile produces no dip at any tier', () => {
  const haloOpacityAt = (tier: 'full' | 'reduced' | 'off', haloOpacity: number): string => {
    expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
    expect(motionCss.length, 'motion.css imported as an empty string').toBeGreaterThan(0);
    const style = document.createElement('style');
    style.textContent = `${cardCss}\n${motionCss}`;
    document.head.append(style);
    document.documentElement.setAttribute('data-effects-tier', tier);
    const container = draw({ haloOpacity });
    const halo = container.querySelector('.cdt-card-halo');
    if (halo === null) throw new Error('the card mounted no .cdt-card-halo');
    return getComputedStyle(halo).opacity;
  };

  it('is clamped at reduced and at off, on an element the clamp really selects', () => {
    expect(haloOpacityAt('full', 0.8)).toContain('--cdt-halo-opacity');
    cleanup();
    expect(haloOpacityAt('reduced', 0.8)).toBe('1');
    cleanup();
    expect(haloOpacityAt('off', 0.8)).toBe('1');
  });

  it('is never a candidate, so the driver never picks it in the first place', async () => {
    const { flickerEligible } = await import('../motion/flicker');
    for (const signal of ['neglected', 'abandoned'] as const) {
      expect(flickerEligible(row({ conditionSignal: signal }))).toBe(false);
    }
    // The counter-case: the same band on a present copy is eligible, so the rule above is the
    // location and not the band.
    expect(
      flickerEligible({ conditionSignal: 'neglected', isReference: false, presence: 'present' }),
    ).toBe(true);
  });
});
