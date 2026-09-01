import { expect, test } from 'vitest';
import { TOKENS } from '../theme/tokens';
// `?raw` rather than node:fs: the renderer project carries no Node types, and under jsdom
// `import.meta.url` is not a file URL. The dom project processes these files so the query
// returns the real text — Vitest otherwise stubs them to an empty module, `?raw` included.
import CSS from './firstRun.css?raw';
import vocabularyJson from '../styles/css-vocabulary.json?raw';

const VOCABULARY = JSON.parse(vocabularyJson) as {
  readonly durationsMs: readonly number[];
  readonly timingFunctions: readonly string[];
  readonly exemptColourLiterals: readonly string[];
};

// A gate that reads an empty file passes every assertion below vacuously. Two suites in this
// repository have already shipped green over a `?raw` import that resolved to ''.
test('the stylesheet was actually read', () => {
  expect(CSS.length).toBeGreaterThan(2000);
  expect(vocabularyJson.length).toBeGreaterThan(200);
});

// §8.7 ends with "the stylesheet's remaining literals are zero — which is what makes the
// prohibition testable instead of aspirational". `scripts/check-style-tokens.mjs` enforces that
// across the tree; this asserts it for this file against the typed token table, so a `var()`
// naming a token that does not exist fails here rather than painting nothing at runtime.
test('every colour is a token and every token named exists', () => {
  const named = [...CSS.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1]!.slice(2));
  expect(named.length).toBeGreaterThan(20);
  for (const name of named) {
    expect(Object.keys(TOKENS), `var(--${name})`).toContain(name);
  }
  const exempt = new Set(VOCABULARY.exemptColourLiterals);
  for (const [literal] of CSS.matchAll(/#[0-9a-f]{3,8}\b|rgba?\([^)]*\)|hsla?\([^)]*\)/gi)) {
    expect(exempt, `colour literal ${literal}`).toContain(literal);
  }
});

// §11.6: the stylesheet declares no duration and no timing function the spec does not state.
// `styles/css-vocabulary.json` is the single owner of both sets — restating them here is the
// "one value stated twice drifts" defect, and a CSS formatter rewriting `.5s` as `0.5s` would
// break a string comparison while saying nothing about §11.6.
test('no duration and no curve outside the stated set', () => {
  const durations = new Set(VOCABULARY.durationsMs);
  const seen: number[] = [];
  for (const [, amount, unit] of CSS.matchAll(/(?<![\w.-])(\d+(?:\.\d+)?|\.\d+)(ms|s)(?![\w-])/g)) {
    const ms = unit === 's' ? Number(amount) * 1000 : Number(amount);
    seen.push(ms);
    expect(durations, `duration ${amount!}${unit!}`).toContain(ms);
  }
  expect(seen.length).toBeGreaterThan(5);

  // A curve is four numbers, not a spelling: the formatter writes `cubic-bezier(0.2, 0.85, …)`
  // where §11.6 writes `cubic-bezier(.2,.85,…)`.
  const key = (v: string): string => v.replace(/\s+/g, '').replace(/\b0\.(\d)/g, '.$1');
  const curves = new Set(VOCABULARY.timingFunctions.map(key));
  for (const [curve] of CSS.matchAll(/cubic-bezier\([^)]*\)/g)) {
    expect(curves, `curve ${curve}`).toContain(key(curve));
  }
});

// §11.6: no animation declares `infinite`, except the first-run scan beam, which is bounded by
// the scan it reports.
test('the scan beam is the only infinite animation', () => {
  const infinite = [...CSS.matchAll(/animation:[^;]*infinite/g)].map((m) => m[0]);
  expect(infinite).toHaveLength(1);
  expect(infinite[0]).toContain('frBeam');
});

// R35(b): viewIn, panelIn and turnIn belong to plan 12's styles/base.css. A copy here would
// silently override that declaration and would sit outside motion.css's tier clamp, which cannot
// reduce or disable an animation declared in a screen's own stylesheet. The uses stay; only the
// declarations moved.
test('the shared entry keyframes are used here and declared elsewhere', () => {
  const declared = [...CSS.matchAll(/@keyframes\s+([\w-]+)/g)].map((m) => m[1]).sort();
  expect(declared).toEqual(['frBeam', 'materialise']);
  expect(CSS).toContain('animation: viewIn');
  expect(CSS).toContain('animation: panelIn');
});

// §10.3a: the beam is decorative and its period is unrelated to the walk. A beam that took
// pointer events would also be a control the user could press by accident.
test('the beam takes no pointer events', () => {
  expect(CSS).toMatch(/\.cdt-fr-beam\b[^}]*pointer-events:\s*none/s);
});

// §8.7: --text-4 and --text-5 are ornament only. §10 assigns --text-4 exactly three slots — an
// unticked root's path (§10.1b), the HITS/PROJECTS unit beneath the count (§10.1b) and the
// reveal footer (§10.4a), which is the one place a grey snaps *down* because the raised
// SHOW WORKING label now says the same thing on all six panels. --text-5 gets none.
test('the two ornament greys appear only where §10 assigns them', () => {
  const ORNAMENT_SLOTS = /cdt-fr-path|cdt-fr-count-unit|cdt-fr-footer/;
  let seen = 0;
  for (const [, selector] of CSS.matchAll(/([^}@/]*)\{[^}]*--text-4[^}]*\}/g)) {
    seen += 1;
    expect(selector, '--text-4').toMatch(ORNAMENT_SLOTS);
  }
  expect(seen).toBeGreaterThan(0);
  expect(CSS).not.toContain('--text-5');
});

// §10.4a: a 260px floor fits three tracks and never four, so six panels lay out 3+3. At 200px
// six lay out 4+2, and a two-item row under a full one reads as two panels that failed to load.
test('the reveal grid floor is 260px', () => {
  expect(CSS).toMatch(/\.cdt-fr-panels\b[^}]*minmax\(\s*260px\s*,\s*1fr\s*\)/s);
  expect(CSS).not.toMatch(/minmax\(\s*200px/);
});

// §10.3a: the scan tile carries a 1px frame, a 1px-inset plate and nothing else. It is not a
// card and must not grow into one.
test('the scan tile has no card furniture', () => {
  const tile = CSS.slice(CSS.indexOf('.cdt-fr-tile'), CSS.indexOf('.cdt-fr-skip'));
  expect(tile.length).toBeGreaterThan(200);
  expect(tile).toMatch(/aspect-ratio:\s*2\s*\/\s*3/);
  expect(tile).not.toContain('clip-path');
  expect(tile).not.toContain('--chamfer');
});

// §11.6: a static equivalent at every tier. At `reduced` and `off` the beam does not travel and
// the tile does not scale — the design's motion contract, not an accessibility afterthought.
test('both first-run animations stop at the two lower tiers', () => {
  for (const tier of ['reduced', 'off']) {
    expect(CSS).toMatch(new RegExp(`\\[data-effects-tier=['"]${tier}['"]\\]`));
  }
  const reduced = CSS.slice(CSS.search(/\[data-effects-tier=['"]reduced['"]\]/));
  expect(reduced).toMatch(/animation:\s*none/);
});
