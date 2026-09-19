import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * §11.6's tier clamp, **derived from `motion.css` rather than enumerated**.
 *
 * `AC-50-zero-animation` carried a hand-maintained list of the class names the clamp selects. It
 * asserted **nine** against a real ten, was corrected to ten, and §33 and §34 then moved it twice
 * more from two sections that cannot see each other. **Three moves is the argument for deriving
 * it**: this file parses the stylesheet, prints the set and its size, and fails at zero. No count
 * is written into any assertion here, and none belongs in a criterion.
 *
 * **Which rule group a name joins *is* its tier contract**, so the three groups are kept apart.
 * A test that only checked membership in the union would pass with `.cdt-surge` in the wrong one,
 * and the wrong group is the dangerous half: an inert effect is a missing payoff, an unclamped
 * one is an accessibility failure on a user who asked their operating system for reduced motion.
 *
 * The file is read with `node:fs`, not imported as `?raw`. `app/test/**` runs in vitest's **node**
 * project, whose config carries no `css.include` — a `?raw` CSS import there resolves to the
 * empty string and every assertion below would pass against nothing.
 */
const MOTION_CSS = fileURLToPath(new URL('../src/renderer/styles/motion.css', import.meta.url));

export interface ClampSets {
  readonly displayNone: ReadonlySet<string>;
  readonly noTransform: ReadonlySet<string>;
  readonly clamped: ReadonlySet<string>;
  readonly all: ReadonlySet<string>;
}

/**
 * Every class name a `[data-effects-tier=…]` rule selects, bucketed by what the rule declares.
 *
 * `clamped` is a transition with a **duration**, which is what §11.6 means by clamping. The
 * `transition: none` rules declare the same property and clamp nothing, so they are deliberately
 * not in that bucket — landing a name there produces no transition at `reduced` at all, which
 * resolves as a plausible style and is exactly the mistake a line-range edit makes.
 */
export function clampClassNames(css: string): ClampSets {
  const withoutComments = css.replace(/\/\*[\s\S]*?\*\//gu, '');
  const displayNone = new Set<string>();
  const noTransform = new Set<string>();
  const clamped = new Set<string>();
  const all = new Set<string>();

  const rule = /([^{}]+)\{([^{}]*)\}/gu;
  let match = rule.exec(withoutComments);
  while (match !== null) {
    const selectors = match[1] ?? '';
    const body = match[2] ?? '';
    const names = new Set<string>();
    for (const selector of selectors.split(',')) {
      if (!selector.includes('[data-effects-tier')) continue;
      for (const found of selector.matchAll(/\.([a-z][a-z0-9-]*)/gu)) {
        const name = found[1];
        if (name !== undefined) names.add(name);
      }
    }
    if (names.size > 0) {
      const declares = (property: string, value: string): boolean =>
        new RegExp(`(^|;)\\s*${property}\\s*:\\s*${value}\\s*(;|$)`, 'u').test(body.trim());
      const isClamped = /(^|;)\s*transition\s*:\s*[a-z-]+\s+\d/u.test(body.trim());
      for (const name of names) {
        all.add(name);
        if (declares('display', 'none')) displayNone.add(name);
        if (declares('transform', 'none')) noTransform.add(name);
        if (isClamped) clamped.add(name);
      }
    }
    match = rule.exec(withoutComments);
  }

  return { displayNone, noTransform, clamped, all };
}

function sets(): ClampSets {
  const css = readFileSync(MOTION_CSS, 'utf8');
  expect(css.length, 'motion.css read as an empty string').toBeGreaterThan(0);
  return clampClassNames(css);
}

describe('the tier clamp, derived from motion.css', () => {
  it('reads a non-empty class set and prints it', () => {
    const { all, displayNone, noTransform, clamped } = sets();
    console.error(
      `AC-P3-33-8 clamp set (${String(all.size)}): ${[...all].sort().join(' ')}\n` +
        `  display:none  (${String(displayNone.size)}): ${[...displayNone].sort().join(' ')}\n` +
        `  transform:none(${String(noTransform.size)}): ${[...noTransform].sort().join(' ')}\n` +
        `  clamped       (${String(clamped.size)}): ${[...clamped].sort().join(' ')}`,
    );
    // A gate whose passing run scans zero files is a failing gate.
    expect(all.size, 'the clamp parser derived no class name at all').toBeGreaterThan(0);
  });

  it('fails at zero rather than reporting an empty set as clean', () => {
    const empty = clampClassNames('');
    expect(empty.all.size).toBe(0);
    expect(empty.displayNone.size).toBe(0);
    expect(empty.noTransform.size).toBe(0);
    expect(empty.clamped.size).toBe(0);
  });

  /**
   * **`AC-P3-33-8`.** §33.5 rules the layers **render** at `reduced` — identical set, identical
   * positions, opacity only, no transform, so no motes, no crack-weld travel, no debris.
   */
  it('puts .cdt-decay in the transform:none rule and in the clamped-transition rule', () => {
    const { displayNone, noTransform, clamped } = sets();
    expect(noTransform.has('cdt-decay'), '.cdt-decay must render with no transform').toBe(true);
    expect(clamped.has('cdt-decay'), '.cdt-decay must carry a clamped opacity transition').toBe(
      true,
    );
    expect(displayNone.has('cdt-decay'), 'no tier removes a layer').toBe(false);
  });

  /**
   * §34.8 — *a light front expanding from a point is a travelling highlight* — and
   * `AC-P3-34-13` requires `reduced` and `off` to resolve `display: none` for the surge.
   *
   * **§33.10 instructs both names into the same rules and is wrong** (R112). p3-33 owns this file
   * for phase 3 and applies §34's placement for §34's element; p3-34 lands the element the entry
   * selects. This lane's claim is set membership, which is all it can decide — the resolved-style
   * verification is `AC-P3-34-13`, p3-34's, because p3-34 mounts the element.
   */
  it('puts .cdt-surge in the display:none rule and in neither of the other two', () => {
    const { displayNone, noTransform, clamped } = sets();
    expect(displayNone.has('cdt-surge'), 'the surge is absent below full').toBe(true);
    expect(noTransform.has('cdt-surge'), 'an unclamped surge is the accessibility failure').toBe(
      false,
    );
    expect(clamped.has('cdt-surge')).toBe(false);
  });

  it('keeps the groups disjoint where §11.6 says they are', () => {
    const { displayNone, noTransform } = sets();
    for (const name of displayNone) {
      expect(
        noTransform.has(name),
        `${name} is both absent and transform-clamped, which cannot both be the contract`,
      ).toBe(false);
    }
  });
});
