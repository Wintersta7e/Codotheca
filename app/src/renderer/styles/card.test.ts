import { afterEach, describe, expect, it } from 'vitest';
import { GRID_TILE_BANDS, HERO_BANDS } from '../card/geometry';
// `?raw` rather than node:fs: the renderer project carries no Node types by design, and under
// jsdom `import.meta.url` is not a file URL. The dom project processes these stylesheets so the
// query returns the real text — Vitest otherwise stubs CSS to an empty module, `?raw` included.
import css from './card.css?raw';
import motionCss from './motion.css?raw';

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

/**
 * §11.6's clamp, resolved rather than read.
 *
 * `motion.css` selects nine class names — every one of them declared by *this* stylesheet — so
 * until the card existed the whole tier clamp matched zero elements. Asserting that a rule is
 * present in the stylesheet text cannot tell the two apart: rename one class and the text still
 * contains both halves while nothing clamps. That is R36 exactly (a gate asserting an attribute
 * the product never set, so every clamp was inert and green), one level further in — so this
 * mounts the two stylesheets together and reads what the cascade actually resolves to.
 */
const CARD_FIXTURE = `
  <div class="cdt-card-frame" data-hovered="true">
    <div class="cdt-card-halo"></div>
    <div class="cdt-bloom"></div>
    <div class="cdt-card" data-hovered="true">
      <div class="cdt-plate">
        <div class="cdt-bracket"></div>
        <div class="cdt-scanline"></div>
        <div class="cdt-dot"></div>
        <div class="cdt-strip"></div>
      </div>
    </div>
  </div>`;

interface Resolved {
  readonly cardBackgroundImage: string;
  readonly cardBackgroundColor: string;
  readonly cardTransform: string;
  readonly cardTransition: string;
  readonly plateBackgroundImage: string;
  readonly bracketDisplay: string;
  readonly scanlineDisplay: string;
  readonly dotTransform: string;
  readonly stripTransform: string;
  readonly haloOpacity: string;
}

/** Both stylesheets, in the order `main.tsx` imports them: card first, motion last. */
function resolveAt(tier: 'full' | 'reduced' | 'off'): Resolved {
  const style = document.createElement('style');
  style.textContent = `${css}\n${motionCss}`;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', tier);
  document.body.innerHTML = CARD_FIXTURE;

  const at = (selector: string): CSSStyleDeclaration => {
    const node = document.querySelector(selector);
    // The fixture must actually carry the element, or every assertion below reads a default and
    // passes against nothing — the failure mode this whole block exists to catch.
    if (node === null) throw new Error(`fixture has no ${selector}`);
    return getComputedStyle(node);
  };

  return {
    cardBackgroundImage: at('.cdt-card').backgroundImage,
    cardBackgroundColor: at('.cdt-card').backgroundColor,
    cardTransform: at('.cdt-card').transform,
    cardTransition: at('.cdt-card').transition,
    plateBackgroundImage: at('.cdt-plate').backgroundImage,
    bracketDisplay: at('.cdt-bracket').display,
    scanlineDisplay: at('.cdt-scanline').display,
    dotTransform: at('.cdt-dot').transform,
    stripTransform: at('.cdt-strip').transform,
    haloOpacity: at('.cdt-card-halo').opacity,
  };
}

afterEach(() => {
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
  document.body.innerHTML = '';
});

describe('the motion tier clamp resolves against these class names', () => {
  it('declares every class motion.css clamps, or the clamp selects nothing', () => {
    // The cascade assertions below mount a fixture carrying these names, so they prove the clamp
    // WINS — they cannot prove this stylesheet spells the names the same way, because the fixture
    // would keep matching motion.css after a rename here. This is that other half, and it is the
    // half a rename breaks: motion.css names nine classes and every one is declared by this file.
    const clamped = [...new Set(motionCss.match(/\.cdt-[a-z-]+/g) ?? [])];
    expect(clamped.length).toBeGreaterThan(0);
    for (const name of clamped) {
      expect(body, `${name} is clamped by motion.css and declared by no card rule`).toContain(name);
    }
  });

  it('leaves every effect standing at full', () => {
    const full = resolveAt('full');
    expect(full.cardBackgroundImage).toContain('--cdt-led');
    expect(full.cardTransform).toBe('translateY(-7px) scale(1.025)');
    expect(full.plateBackgroundImage).toContain('--cdt-greebling');
    expect(full.bracketDisplay).not.toBe('none');
    expect(full.scanlineDisplay).not.toBe('none');
    expect(full.dotTransform).toBe('scale(1.5)');
  });

  it('drops the travelling highlights and every transform at reduced', () => {
    const reduced = resolveAt('reduced');
    expect(reduced.cardBackgroundImage).toBe('none');
    expect(reduced.cardTransform).toBe('none');
    expect(reduced.bracketDisplay).toBe('none');
    expect(reduced.scanlineDisplay).toBe('none');
    expect(reduced.dotTransform).toBe('none');
    expect(reduced.stripTransform).toBe('none');
    // The greebling goes with the travelling layers; the plate itself stays.
    expect(reduced.plateBackgroundImage).not.toContain('--cdt-greebling');
    expect(reduced.plateBackgroundImage).toContain('--cdt-plate');
    // The flicker rides the tier and gets no switch of its own (§11.3a).
    expect(reduced.haloOpacity).toBe('1');
  });

  it('clamps the same set at off and stops every transition', () => {
    const off = resolveAt('off');
    expect(off.cardBackgroundImage).toBe('none');
    expect(off.cardTransform).toBe('none');
    expect(off.bracketDisplay).toBe('none');
    expect(off.cardTransition).toBe('none');
    expect(off.haloOpacity).toBe('1');
  });

  it('never gates a state — the hovered frame survives every tier', () => {
    // §11.6: the tier gates transitions and travelling highlights, never states. A hovered card
    // takes the jewel frame at `off` too, instantly.
    for (const tier of ['full', 'reduced', 'off'] as const) {
      expect(resolveAt(tier).cardBackgroundColor).toContain('--cdt-jewel');
    }
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
