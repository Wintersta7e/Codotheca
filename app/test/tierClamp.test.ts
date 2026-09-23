import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { clampClassNames, type ClampSets } from '../../scripts/lib/motion-clamp.mjs';

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
 *
 * [p3] **The parse has one owner, `scripts/lib/motion-clamp.mjs`**, which §34.8's standing checker
 * (`scripts/check-motion-clamp.mjs`) reads too. This file asks which rule group a name joins; the
 * checker asks whether every animated class joins one. Two parsers of one stylesheet would be two
 * answers to the same question waiting to disagree.
 */
const MOTION_CSS = fileURLToPath(new URL('../src/renderer/styles/motion.css', import.meta.url));

function sets(): ClampSets {
  const css = readFileSync(MOTION_CSS, 'utf8');
  expect(css.length, 'motion.css read as an empty string').toBeGreaterThan(0);
  return clampClassNames(css);
}

describe('the tier clamp, derived from motion.css', () => {
  it('ac_p3_33_8 reads a non-empty class set and prints it', () => {
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

  it('ac_p3_33_8 fails at zero rather than reporting an empty set as absent', () => {
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
  it('ac_p3_33_8 puts .cdt-decay in the transform:none rule and in the clamped rule', () => {
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
  it('ac_p3_33_8 puts .cdt-surge in the display:none rule and in neither other', () => {
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
