import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { SceneHash } from '../../generated/protocol';
import { HeroFrame } from '../card/HeroFrame';
import { rowFixture } from '../project/testFixtures';
import cardCss from '../styles/card.css?raw';
import motionCss from '../styles/motion.css?raw';
import { RESTORATION_SURGE_MS } from './envelope';
import type { Selection } from './select';
import { Surge } from './Surge';

type Tier = 'full' | 'reduced' | 'off';

/** The element under test. The `.cdt-plate`-for-`.cdt-specular` failure is asking the wrong one. */
const SUBJECT = '.cdt-surge';

afterEach(() => {
  cleanup();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
});

/** The three renderings a surge can mount — each is held to the same tier contract. */
const RENDERINGS: readonly [string, Selection, { fx: number; fy: number } | null][] = [
  ['point', { kind: 'layer', layer: 'rust' }, { fx: 0.25, fy: 0.5 }],
  ['wipe', { kind: 'layer', layer: 'dust' }, null],
  ['whole', { kind: 'whole' }, null],
];

/**
 * Mounted through the **product's own path** — `HeroFrame`'s surge slot, so the element sits
 * inside `.cdt-card` as it does on the page — and resolved on that real element. The pattern is
 * `card/blueprint.test.tsx`'s, copied rather than re-invented.
 */
function mountAt(
  tier: Tier,
  selection: Selection,
  origin: { fx: number; fy: number } | null,
): CSSStyleDeclaration {
  // Both stylesheets asserted non-empty FIRST: a `?raw` import that resolved to "" would make
  // every assertion below read a default and pass against nothing.
  expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
  expect(motionCss.length, 'motion.css imported as an empty string').toBeGreaterThan(0);
  const style = document.createElement('style');
  style.textContent = `${cardCss}\n${motionCss}`;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', tier);
  render(
    <HeroFrame
      row={rowFixture({ artSceneHash: 'aa11bb22' as unknown as SceneHash })}
      heroSrc=""
      halo={{ shadow: null, opacity: 1 }}
      chips={[]}
      pin={null}
      surge={() => <Surge selection={selection} origin={origin} onEnd={() => undefined} />}
    >
      {null}
    </HeroFrame>,
  );
  const node = document.querySelector(SUBJECT);
  if (node === null) throw new Error(`no ${SUBJECT} element mounted`);
  expect(node.closest('.cdt-card'), 'the element sits outside the card subtree').not.toBeNull();
  return getComputedStyle(node);
}

function reset(): void {
  cleanup();
  document.head.querySelectorAll('style').forEach((n) => {
    n.remove();
  });
}

/** Milliseconds out of a resolved duration, **as a number**. The formatter may rewrite ms as s. */
function durationMs(value: string): number {
  const match = /(\d*\.?\d+)(ms|s)\b/u.exec(value);
  if (match === null) return Number.NaN;
  const amount = Number.parseFloat(match[1] ?? '');
  return match[2] === 's' ? amount * 1000 : amount;
}

/**
 * **`AC-P3-34-13`** — at `reduced` and `off` the surge is absent; at `full` it plays its one
 * envelope. A light front expanding from a point is a travelling highlight, so it goes with the
 * specular sweep, the brackets and the scan line — and an unclamped one would run more than an
 * order of magnitude past the reduced clamp, on a user who asked for reduced motion.
 *
 * **No assertion here matches stylesheet text.** Which rule group `motion.css` puts the name in is
 * the contract, and only the resolved style on a mounted element can say which one it is.
 */
describe('ac p3 34 13 — the surge resolved on a real mounted element at every tier', () => {
  it('ac_p3_34_13 is display:none at reduced and at off, for every rendering', () => {
    let resolved = 0;
    for (const [name, selection, origin] of RENDERINGS) {
      for (const tier of ['reduced', 'off'] as const) {
        expect(mountAt(tier, selection, origin).display, `${name} painted at ${tier}`).toBe('none');
        reset();
        resolved += 1;
      }
    }
    console.error(`AC-P3-34-13 resolved below full: ${String(resolved)} element(s)`);
    expect(resolved).toBeGreaterThan(0);
  });

  it('ac_p3_34_13 plays at full, over the envelope §11.6 owns, compared as a number', () => {
    let resolved = 0;
    for (const [name, selection, origin] of RENDERINGS) {
      const style = mountAt('full', selection, origin);
      expect(style.display, `${name} is absent at full`).not.toBe('none');
      expect(style.animationName, `${name} names no animation`).not.toBe('none');
      expect(durationMs(style.animationDuration), `${name}'s envelope`).toBe(RESTORATION_SURGE_MS);
      reset();
      resolved += 1;
    }
    console.error(`AC-P3-34-13 resolved at full: ${String(resolved)} element(s)`);
    expect(resolved).toBeGreaterThan(0);
  });
});
