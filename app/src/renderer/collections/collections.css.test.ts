import { describe, expect, it } from 'vitest';
import css from './collections.css?raw';

/**
 * Scoped to `styles/`, `vitest.config.ts`'s `css.include` returns the **empty string** for a
 * `?raw` import outside it and every assertion below passes vacuously. Two lanes hit that
 * independently, so the length is the first thing asserted and everything else depends on it.
 */
describe('collections.css is actually read', () => {
  it('is not the empty string vitest substitutes for an unprocessed stylesheet', () => {
    expect(css.length).toBeGreaterThan(400);
    expect(css).toContain('.cdt-collection');
  });
});

describe('collections.css', () => {
  it('declares no literal colour — every colour is a token', () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\brgba?\(/);
    expect(css).not.toMatch(/\boklch\(/);
  });

  // §8.8: the chrome is §8.0b's and is not restated. A padding or a ground here is the
  // duplication the section exists to refuse.
  it('restates none of §8.0b’s chip box', () => {
    const open = css.indexOf('.cdt-collection {');
    expect(open).toBeGreaterThanOrEqual(0);
    const chip = css.slice(open, css.indexOf('}', open));
    expect(chip).not.toMatch(/\bpadding\b/);
    expect(chip).not.toMatch(/\bbackground\b/);
    expect(chip).not.toMatch(/font-size/);
  });

  it('pins the armed ground and drops the hover fill', () => {
    expect(css).toMatch(/\[data-armed=(['"])true\1\][^{]*\{[^}]*var\(--surface-2\)/s);
  });

  it('outlines broken in --fail-hot and never in --fail', () => {
    expect(css).toMatch(/\[data-state=(['"])broken\1\][^{]*\{[^}]*var\(--fail-hot\)/s);
  });

  it('reveals the × on hover and on keyboard focus, not on hover alone', () => {
    expect(css).toMatch(/:focus-within[^{]*\.cdt-collection-remove/);
  });

  // §11.7: `outline: none` with no replacement indicator in the same rule is a build failure.
  it('replaces every outline it removes, in the same rule', () => {
    let checked = 0;
    for (const rule of css.split('}')) {
      if (/outline:\s*none/.test(rule)) {
        checked += 1;
        expect(rule).toMatch(/box-shadow:/);
      }
    }
    // A pass that inspected no rule proves nothing about the rule that removes an outline.
    expect(checked).toBeGreaterThan(0);
  });

  // Criterion 50: at tier `off` nothing transitions, and the state is still readable.
  //
  // The attribute is `data-effects-tier`, set on the document element by `motion/tier.ts`.
  // `motion.css`'s blanket `off` rule stops `animation` and not `transition`, so this rule is
  // additive rather than a second copy — and a rule written against `data-effects` would select
  // nothing at all while looking exactly like this one.
  it('confines its transitions to the effects-tier attribute the product really sets', () => {
    expect(css).not.toMatch(/\[data-effects=/);
    expect(css).toMatch(/\[data-effects-tier=(['"])off\1\][^{]*\{[^}]*transition:\s*none/s);
  });

  // §8.7: decision-carrying text is --text-3 or lighter. --text-4 and --text-5 are ornament.
  it('sets no decision-carrying text below the floor', () => {
    expect(css).not.toContain('--text-4');
    expect(css).not.toContain('--text-5');
  });
});
