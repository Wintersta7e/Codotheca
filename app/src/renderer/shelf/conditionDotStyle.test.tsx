/**
 * §5.4a's dot on the shelf's two text-row surfaces — the list's leading slot (§8.0c) and the
 * reference tail — read as the browser resolves it, under the real stylesheets. The sibling
 * tests read the custom properties the row hands in; those were set all along while no rule
 * consumed them, so the column painted nothing.
 *
 * jsdom substitutes no `var()`: a computed `background-color` comes back as the reference
 * itself. `resolved` follows that reference to the custom property computed on the same element,
 * which is the substitution a browser performs. A `border` built from `var()` is not readable at
 * all here, so the ring is checked by the band data and the fill by the resolved style.
 */
import { cleanup, render } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import baseCss from '../styles/base.css?raw';
import motionCss from '../styles/motion.css?raw';
import { DOT_SIZE_PX, conditionDot, type ConditionSignal } from '../derive/condition.js';
import { noop } from '../noop.js';
import { ListView } from './ListView.js';
import { ReferenceTail } from './ReferenceTail.js';
import type { ShelfRow } from './row.js';

afterEach(cleanup);

const style = document.createElement('style');
beforeAll(() => {
  expect(baseCss.length, 'base.css?raw imported as an empty string').toBeGreaterThan(0);
  expect(motionCss.length, 'motion.css?raw imported as an empty string').toBeGreaterThan(0);
  style.textContent = `${baseCss}\n${motionCss}`;
  document.head.append(style);
});
afterAll(() => {
  style.remove();
});

const row = (signal: ConditionSignal, isReference: boolean): ShelfRow =>
  ({
    id: 1,
    name: 'atlas',
    primaryLanguage: 'Rust',
    branch: 'main',
    sizeTrackedBytes: null,
    conditionSignal: signal,
    isArchived: false,
    isReference,
    interruptedOp: null,
    refstateObservedAt: 0,
    isDirty: null,
    worktreeObservedAt: null,
    ahead: null,
    behind: null,
    fetchHeadAt: null,
    createdAt: 0,
    acknowledgedAt: 0,
    completionLit: null,
    completionApplicable: null,
    primaryLocation: { id: 10, pathDisplay: '/w/atlas' },
  }) as unknown as ShelfRow;

const VAR = /^var\(\s*(--[\w-]+)\s*\)$/;

/** A computed longhand with its one `var()` reference substituted, as a browser would. */
function resolved(element: Element, property: string): string {
  const computed = getComputedStyle(element);
  const value = computed.getPropertyValue(property).trim();
  const reference = VAR.exec(value);
  return reference?.[1] === undefined ? value : computed.getPropertyValue(reference[1]).trim();
}

function mountDot(surface: ReactElement, tier: string | null): HTMLElement {
  const { container } = render(
    tier === null ? surface : <div data-effects-tier={tier}>{surface}</div>,
  );
  const dot = container.querySelector<HTMLElement>('.cdt-condition-dot');
  if (dot === null) throw new Error('no condition dot mounted');
  return dot;
}

const SURFACES: readonly [string, (signal: ConditionSignal) => ReactElement][] = [
  [
    'list row',
    (signal) => (
      <ListView
        rows={[row(signal, false)]}
        now={0}
        firstRunCompletedAt={0}
        selectedId={null}
        peek={null}
        onActivate={noop}
        onOpen={noop}
      />
    ),
  ],
  ['reference tail', (signal) => <ReferenceTail rows={[row(signal, true)]} onOpen={noop} />],
];

describe('the shelf condition dot paints', () => {
  for (const [name, surface] of SURFACES) {
    it(`resolves to a ${String(DOT_SIZE_PX.listRow)}px disc in its band on the ${name}`, () => {
      const dot = mountDot(surface('dormant'), null);
      const band = conditionDot({
        signal: 'dormant',
        isReference: name === 'reference tail',
        isArchived: false,
      });
      const computed = getComputedStyle(dot);
      // An inline span with no rule computes `display: inline` and has no box at any size.
      expect(computed.display).toBe('inline-block');
      expect(computed.borderRadius).toBe('50%');
      expect(computed.width).toBe(`${String(DOT_SIZE_PX.listRow)}px`);
      expect(computed.height).toBe(`${String(DOT_SIZE_PX.listRow)}px`);
      expect(resolved(dot, 'background-color')).toBe(band?.fill ?? 'transparent');
    });

    // §11.6's clamp selects `.cdt-dot`, the card's dot; this one carries no motion to clamp and
    // no tier may hide it — a dot that vanished at `off` would read as "not computed".
    it(`keeps the same resolved dot at every effects tier on the ${name}`, () => {
      const full = mountDot(surface('dormant'), null);
      const expected = {
        display: getComputedStyle(full).display,
        fill: resolved(full, 'background-color'),
      };
      cleanup();
      for (const tier of ['reduced', 'off']) {
        const dot = mountDot(surface('dormant'), tier);
        expect({
          display: getComputedStyle(dot).display,
          fill: resolved(dot, 'background-color'),
        }).toEqual(expected);
        cleanup();
      }
    });
  }
});
