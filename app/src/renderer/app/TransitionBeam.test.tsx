import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import motionCss from '../styles/motion.css?raw';
import transitionCss from '../styles/transition.css?raw';
import { makeProjectRow } from '../testing/projectRow';
import { TransitionBeam } from './TransitionBeam';

/**
 * The measurement ticks that race along the opening beam (`Codotheca v7 Shelf.dc.html:1073`),
 * resolved on the element `TransitionBeam` really renders rather than on a hand-written fixture.
 */
const row = makeProjectRow();

afterEach(() => {
  cleanup();
  document.head.innerHTML = '';
  document.documentElement.removeAttribute('data-effects-tier');
});

function tickAt(
  tier: 'full' | 'reduced' | 'off',
  kind: 'opening' | 'closing' = 'opening',
): CSSStyleDeclaration {
  expect(transitionCss.length, 'transition.css arrived empty').toBeGreaterThan(500);
  expect(motionCss.length, 'motion.css arrived empty').toBeGreaterThan(500);
  const style = document.createElement('style');
  style.textContent = `${transitionCss}\n${motionCss}`;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', tier);
  const { container } = render(<TransitionBeam phase={{ kind, id: row.id }} rows={[row]} />);
  const tick = container.querySelector('.cdt-beam-stage > .cdt-beam-tick');
  if (tick === null) throw new Error('the beam renders no ticks');
  return getComputedStyle(tick);
}

describe('the opening beam carries its measurement ticks', () => {
  it('races them along the beam at `full`', () => {
    expect(tickAt('full').animation).toMatch(/^beamTick\b/u);
  });

  it('does not run them on the way back — the prototype draws them opening only', () => {
    const tick = tickAt('full', 'closing');
    expect(tick.animation).not.toContain('beamTick');
    expect(tick.opacity).toBe('0');
  });

  for (const tier of ['reduced', 'off'] as const) {
    it(`draws nothing at ${tier}`, () => {
      const tick = tickAt(tier);
      expect(tick.animation).not.toContain('beamTick');
      expect(tick.opacity).toBe('0');
    });
  }
});
