// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';

import cardCss from './card.css?raw';
import transitionCss from './transition.css?raw';

/**
 * §8.5.1's gesture, resolved on real elements at every tier rather than matched in the stylesheet.
 *
 * R36 is why: a gate that greps a stylesheet passes whether or not the selector ever selects
 * anything, and this project has shipped a motion clamp that was inert for exactly that reason —
 * twice, counting the specular sweep that was clamped on the wrong element.
 */
const FIXTURE = `
  <div class="cdt-card-frame" data-gesture="collapse"></div>
  <div class="cdt-card-frame" id="unfolding" data-gesture="unfold"></div>
  <div class="cdt-card-frame" id="rippling" data-gesture="ripple" style="--cdt-ripple-delay: .34s"></div>
  <div class="cdt-shelf" id="receding" data-gesture="recede"></div>
  <div class="cdt-shelf" id="returning" data-gesture="return"></div>
  <div class="cdt-beam-stage" data-gesture="opening">
    <div class="cdt-beam-line"></div>
    <div class="cdt-beam-flare"></div>
  </div>
  <div class="cp-page" data-gesture="rackout"></div>
  <span class="cdt-era-flare" data-gesture="flare"></span>
  <div class="cdt-card-frame" id="resting"></div>`;

afterEach(() => {
  document.documentElement.removeAttribute('data-effects-tier');
  document.head.innerHTML = '';
  document.body.innerHTML = '';
});

function mountAt(tier: 'full' | 'reduced' | 'off'): void {
  expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
  expect(transitionCss.length, 'transition.css imported as an empty string').toBeGreaterThan(0);
  const style = document.createElement('style');
  style.textContent = `${cardCss}\n${transitionCss}`;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', tier);
  document.body.innerHTML = FIXTURE;
}

/**
 * The keyframe name, read off the `animation` shorthand.
 *
 * **Not `animationName`** — jsdom does not expand the shorthand, so it answers `none` for every
 * element whether or not a rule matched, which made the `reduced` and `off` cases below pass
 * against nothing on the first run of this file. And only the name is compared: the Write hook
 * reformats `.css` on save (`cubic-bezier(.4,.1,.6,1)` becomes `cubic-bezier(0.4, 0.1, 0.6, 1)`),
 * so asserting the whole shorthand as a string would fail on formatting rather than on behaviour.
 */
function animationOf(selector: string): string {
  const node = document.querySelector(selector);
  // The fixture must carry the element, or every assertion below reads a default and passes
  // against nothing — the failure this whole file exists to catch.
  if (node === null) throw new Error(`fixture has no ${selector}`);
  const shorthand = getComputedStyle(node).animation.trim();
  return shorthand === '' ? 'none' : (shorthand.split(/\s+/u)[0] ?? 'none');
}

describe('§8.5.1: every keyframe reaches the element that carries it', () => {
  it('names each one at `full`, on the element the component really renders', () => {
    mountAt('full');
    expect(animationOf('.cdt-card-frame[data-gesture="collapse"]')).toBe('crtCollapse');
    expect(animationOf('#unfolding')).toBe('cardUnfold');
    expect(animationOf('#rippling')).toBe('ripple');
    expect(animationOf('#receding')).toBe('shelfRecede');
    expect(animationOf('#returning')).toBe('shelfReturn');
    expect(animationOf('.cdt-beam-line')).toBe('beamStretch');
    expect(animationOf('.cdt-beam-flare')).toBe('beamFlare');
    expect(animationOf('.cp-page[data-gesture="rackout"]')).toBe('rackOut');
    expect(animationOf('.cdt-era-flare')).toBe('stripFlare');
  });

  it('leaves a resting tile alone, so the selector is the gesture and not the class', () => {
    mountAt('full');
    expect(animationOf('#resting')).not.toBe('crtCollapse');
    expect(animationOf('#resting')).not.toBe('cardUnfold');
  });

  /**
   * The rule must read the offset from the custom property rather than carrying one of its own —
   * nine hard-coded delays would be nine places for §8.5.1's arithmetic to drift from
   * `rippleDelay`, which owns it. The arithmetic itself is covered in `motion/transition.test.ts`.
   */
  it('takes the ripple offset from the custom property and never states one', () => {
    mountAt('full');
    const rule = transitionCss.slice(transitionCss.indexOf("[data-gesture='ripple']"));
    const declaration = rule.slice(0, rule.indexOf('}'));
    expect(declaration).toContain('var(--cdt-ripple-delay)');
  });
});

/**
 * §11.6 drops travelling highlights below `full`, and every part of this gesture is one:
 * `shelfRecede` blurs, `crtCollapse` and `cardUnfold` drive `filter: brightness` to 4, and the
 * beam is light travelling across the screen. There is no clamped version to keep.
 */
describe('§11.6: none of it runs below `full`', () => {
  for (const tier of ['reduced', 'off'] as const) {
    it(`plays nothing at ${tier}`, () => {
      mountAt(tier);
      for (const selector of [
        '.cdt-card-frame[data-gesture="collapse"]',
        '#unfolding',
        '#rippling',
        '#receding',
        '#returning',
        '.cdt-beam-line',
        '.cdt-beam-flare',
        '.cp-page[data-gesture="rackout"]',
        '.cdt-era-flare',
      ]) {
        expect(animationOf(selector), `${selector} still animates at ${tier}`).toBe('none');
      }
    });
  }
});
