import { describe, expect, it } from 'vitest';
import { GRID_TILE_BANDS, HERO_BANDS } from '../card/geometry';
// `?raw` rather than node:fs: the renderer project carries no Node types by design, and under
// jsdom `import.meta.url` is not a file URL. The dom project processes these stylesheets so the
// query returns the real text — Vitest otherwise stubs CSS to an empty module, `?raw` included.
import css from './card.css?raw';

/**
 * Comments are stripped before any structural match. A rule's own explanation legitimately names
 * the selectors and declarations it is about, and a matcher that reads them finds a "rule" that
 * is prose — the same mistake as grepping for a declaration and hitting a plan's gap list.
 */
const body = css.replace(/\/\*[\s\S]*?\*\//g, '');

/**
 * Whitespace- and leading-zero-insensitive, so a reformat cannot fail a *value* assertion. The
 * band tables spell an alpha `/ .13` and a CSS formatter writes `/ 0.13`; the bezel is the eight
 * numbers, not their spelling, which is the same argument `check-style-tokens.mjs` makes for a
 * cubic-bezier. A different number still fails.
 */
const spelling = (text: string): string => text.replace(/\s+/g, ' ').replace(/\b0\.(\d)/g, '.$1');
const flat = spelling(body);

// Every declaration that applies to `selector`, not the first block that mentions it.
// `.cdt-card-halo` is written twice on purpose — once grouped with `.cdt-bloom` for the shared
// box geometry, once alone for its own shadow — so a first-match helper returns the geometry
// block and `toContain('box-shadow')` fails against CSS that is correct. Matching the selector
// as a whole token in the selector list also stops `.cdt-card` matching `.cdt-card-halo`.
const rule = (selector: string): string => {
  const blocks: string[] = [];
  for (const [, selectors, declarations] of body.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    const names = (selectors ?? '').split(',').map((s) => s.trim());
    if (names.some((n) => n === selector || n.endsWith(` ${selector}`))) {
      blocks.push(declarations ?? '');
    }
  }
  if (blocks.length === 0) throw new Error(`no rule for ${selector}`);
  return blocks.join('\n');
};

describe('the chamfer', () => {
  it('clips the card and the plate from the tokens, never a literal radius', () => {
    expect(rule('.cdt-card')).toContain('var(--chamfer-card)');
    expect(body).toContain('var(--chamfer-hero)');
    expect(rule('.cdt-card')).toContain('clip-path: polygon(');
  });
});

describe('the focus indicator survives the clip', () => {
  it('removes the outline and draws an inset ring in the same rule', () => {
    const focus = rule('.cdt-card:focus-visible');
    expect(focus).toContain('outline: none');
    expect(focus).toContain('inset 0 0 0 2px var(--cdt-jewel-ink)');
    expect(focus).toContain('inset 0 0 22px -6px var(--cdt-jewel-55)');
  });

  it('draws the same indicator for selection, because they are one indicator', () => {
    const focusSelector = /\.cdt-card:focus-visible[^{]*\{/.exec(body)?.[0] ?? '';
    expect(focusSelector).toContain("[data-selected='true']");
    expect(focusSelector).toContain("[data-focused='true']");
  });

  it('never puts an outer shadow on the clipped card, which would render nothing', () => {
    // `\b` after `.cdt-card` is a boundary before a hyphen too, so it also matches
    // `.cdt-card-halo` and `.cdt-card-frame` — the two unclipped siblings whose whole job is to
    // carry an outer shadow. The lookahead is what makes this scan the clipped element only.
    let scannedBlocks = 0;
    for (const block of body.split('}')) {
      if (!/\.cdt-card(?![\w-])/.test(block)) continue;
      const shadow = /box-shadow:([^;]*)/.exec(block)?.[1];
      if (shadow === undefined) continue;
      scannedBlocks += 1;
      for (const segment of shadow.split(/,(?![^(]*\))/)) {
        expect(segment.trim().startsWith('inset')).toBe(true);
      }
    }
    // A scan that reached no block is a passing test that proves nothing.
    expect(scannedBlocks).toBeGreaterThan(0);
  });

  it('puts the two outer shadows on the unclipped siblings', () => {
    expect(rule('.cdt-card-halo')).toContain('box-shadow');
    expect(rule('.cdt-bloom')).toContain('box-shadow');
    expect(rule('.cdt-card-halo')).not.toContain('clip-path');
    expect(rule('.cdt-bloom')).not.toContain('clip-path');
  });

  it('gives the glow element the opacity transition §11.6 states, not the bloom curve', () => {
    // §11.6: the flicker is "opacity on a dedicated glow element with its own
    // `transition: opacity 90ms linear`". motion.css already clamps the halo's opacity at
    // reduced and off as if this existed.
    const halo = /\.cdt-card-halo\s*\{([^}]*)\}/.exec(body)?.[1] ?? '';
    expect(halo).toContain('transition: opacity 90ms linear');
    expect(halo).not.toContain('box-shadow 220ms');
  });
});

describe('the tier-gated properties are declarations here, not inline values', () => {
  it('reads the LED run and the plate through custom properties', () => {
    expect(rule('.cdt-card')).toContain('background-image: var(--cdt-led)');
    expect(rule('.cdt-plate')).toContain(
      'background-image: var(--cdt-greebling), var(--cdt-plate)',
    );
  });
});

describe('the bezels match the band tables exactly', () => {
  it('carries the tile bezel and the hero bezel, each once', () => {
    expect(flat).toContain(spelling(GRID_TILE_BANDS.bezel));
    expect(flat).toContain(spelling(HERO_BANDS.bezel));
    expect(GRID_TILE_BANDS.bezel).not.toBe(HERO_BANDS.bezel);
    // …and the normalisation must not have made it permissive: the hero's own inset depth is
    // one of the edges the two tables exist to keep apart.
    expect(flat).not.toContain(spelling(HERO_BANDS.bezel).replace('-30px 40px', '-22px 30px'));
  });
});

describe('the card carries no hex and no roast', () => {
  it('declares every colour through a token or a card-local property', () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toContain('--jewel');
    expect(css).toContain('--cdt-jewel');
  });

  it('carries none of §5.6 roast vocabulary — the card is not where roasting appears', () => {
    for (const banned of ['roast', 'Roast', 'ROAST']) expect(css).not.toContain(banned);
  });
});
