import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import cardCss from '../styles/card.css?raw';
import motionCss from '../styles/motion.css?raw';
import { withProjectDeps } from '../testing/deps';
import { makeProjectRow } from '../testing/projectRow';
import { BRACKET_STAGGER_S } from './interaction';
import { ProjectCard } from './ProjectCard';

/**
 * §7.8's corner brackets, resolved on a real tile rather than a hand-written fixture: the clamp
 * test in `styles/card.test.ts` drew its own `.cdt-bracket` node, so it stayed green while no
 * component mounted one at all.
 *
 * jsdom does not substitute `var()` (measured: `top` computes to `var(--cdt-bracket-at)`), so the
 * inset is read by following that one reference to the custom property computed on the same
 * element, which is the substitution a browser performs.
 */
const SIDES = ['top', 'right', 'bottom', 'left'] as const;
type Side = (typeof SIDES)[number];
const CORNERS: readonly (readonly [string, readonly Side[]])[] = [
  ['top-left', ['top', 'left']],
  ['top-right', ['top', 'right']],
  ['bottom-left', ['bottom', 'left']],
  ['bottom-right', ['bottom', 'right']],
];

afterEach(() => {
  cleanup();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
});

function mountAt(tier: 'full' | 'reduced' | 'off'): HTMLElement {
  expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
  expect(motionCss.length, 'motion.css imported as an empty string').toBeGreaterThan(0);
  const style = document.createElement('style');
  style.textContent = `${cardCss}\n${motionCss}`;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', tier);
  const { container } = render(
    <ProjectCard
      row={makeProjectRow({ conditionSignal: 'dormant' })}
      density={186}
      rendition="card"
      selected={false}
      focused={false}
      now={10_000}
      firstRunCompletedAt={null}
      session={null}
      haloOpacity={1}
      onActivate={vi.fn()}
      onOpen={vi.fn()}
      onTogglePin={vi.fn()}
      onStopSession={vi.fn()}
    />,
    { wrapper: withProjectDeps() },
  );
  return container;
}

function hover(container: HTMLElement): void {
  const card = container.querySelector('.cdt-card');
  if (card === null) throw new Error('the tile mounted no .cdt-card');
  fireEvent.mouseEnter(card);
}

function bracket(container: HTMLElement, corner: string): CSSStyleDeclaration {
  const node = container.querySelector(`.cdt-bracket[data-corner="${corner}"]`);
  if (node === null) throw new Error(`the tile mounted no ${corner} bracket`);
  return getComputedStyle(node);
}

/** The inset a browser would compute for `side`, or `auto` for a side the corner leaves free. */
function inset(style: CSSStyleDeclaration, side: Side): string {
  const raw = style.getPropertyValue(side);
  if (raw !== 'var(--cdt-bracket-at)') return raw;
  return style.getPropertyValue('--cdt-bracket-at').trim();
}

describe('§7.8: the corner brackets mount on the tile and fly in on hover', () => {
  it('mounts exactly four, one per corner, carrying no content', () => {
    const container = mountAt('full');
    const nodes = [...container.querySelectorAll('.cdt-plate .cdt-bracket')];
    expect(nodes.map((node) => node.getAttribute('data-corner'))).toEqual(
      CORNERS.map(([corner]) => corner),
    );
    for (const node of nodes) {
      expect(node.getAttribute('aria-hidden')).toBe('true');
      expect(node.textContent).toBe('');
    }
  });

  it('rests transparent at 18px and lands opaque at 7px, each corner on its own two sides', () => {
    const container = mountAt('full');
    for (const [corner, own] of CORNERS) {
      const rest = bracket(container, corner);
      expect(rest.opacity, `${corner} at rest`).toBe('0');
      expect(rest.display).not.toBe('none');
      for (const side of SIDES) {
        expect(inset(rest, side), `${corner} ${side} at rest`).toBe(
          own.includes(side) ? '18px' : 'auto',
        );
      }
    }
    hover(container);
    for (const [corner, own] of CORNERS) {
      const lit = bracket(container, corner);
      expect(lit.opacity, `${corner} hovered`).toBe('1');
      expect(lit.position).toBe('absolute');
      expect([lit.width, lit.height]).toEqual(['13px', '13px']);
      // Above band 5's scrim, which is positioned with no z-index of its own; under it the
      // bottom pair would be drawn and never seen.
      expect(lit.zIndex).toBe('1');
      for (const side of SIDES) {
        expect(inset(lit, side), `${corner} ${side} hovered`).toBe(
          own.includes(side) ? '7px' : 'auto',
        );
        const width = lit.getPropertyValue(`border-${side}-width`);
        expect(width, `${corner} border-${side}`).toBe(own.includes(side) ? '2px' : '0px');
      }
    }
  });

  it('staggers the four by BRACKET_STAGGER_S, in corner order', () => {
    const container = mountAt('full');
    CORNERS.forEach(([corner], index) => {
      const style = bracket(container, corner);
      expect(style.transitionDelay).toBe('var(--cdt-bracket-delay)');
      expect(style.getPropertyValue('--cdt-bracket-delay').trim()).toBe(BRACKET_STAGGER_S[index]);
    });
  });

  it('is absent below full, hovered or not, on the element that carries it', () => {
    for (const tier of ['reduced', 'off'] as const) {
      const container = mountAt(tier);
      hover(container);
      for (const [corner] of CORNERS) {
        expect(bracket(container, corner).display, `${corner} at ${tier}`).toBe('none');
      }
      cleanup();
    }
  });
});
