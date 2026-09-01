import { describe, expect, it } from 'vitest';
import baseCss from '../styles/base.css?raw';
import motionCss from '../styles/motion.css?raw';
import { OFF_TOKEN_GREYS, tokenValue, type TokenName } from '../theme/tokens.js';
import { isOnScale, type TypeFamily } from '../theme/type.js';
import {
  BACKDROP_KEYFRAME,
  BACKDROP_SCRIM,
  DRAWER_WIDTH_PX,
  PANEL_KEYFRAME,
  PANEL_SHADOW,
  SD,
  backdropAnimation,
  panelAnimation,
} from './styles.js';

/** `var(--name, fallback)` — the fallback must be exactly what the token holds. */
const VAR = /var\(--([a-z0-9-]+),\s*([^)]*)\)/g;
const HEX = /#[0-9a-f]{3,8}\b/gi;

/** §11.3a states these two verbatim and §8.7's ladder holds no alpha ground for either. */
const STATED_VERBATIM = new Set<string>([BACKDROP_SCRIM, PANEL_SHADOW]);

const allValues = (): string[] =>
  Object.values(SD).flatMap((style) => Object.values(style).map((v) => String(v)));

const familyOf = (style: Record<string, unknown>): TypeFamily | null => {
  const family = String(style['fontFamily'] ?? '');
  if (family.includes('--font-display')) return 'display';
  if (family.includes('--font-mono')) return 'mono';
  if (family.includes('--font-body')) return 'body';
  return null;
};

describe('the drawer stylesheet', () => {
  it('is 400px wide and never wider than the window', () => {
    expect(DRAWER_WIDTH_PX).toBe(400);
    expect(SD.panel.width).toBe('400px');
    expect(SD.panel.maxWidth).toBe('100%');
    expect(SD.panel.overflowY).toBe('auto');
  });

  it('writes every var() fallback as the value that token actually holds', () => {
    let seen = 0;
    for (const value of allValues()) {
      for (const [, name, fallback] of value.matchAll(VAR)) {
        seen += 1;
        expect(tokenValue(name as TokenName)).toBe(fallback);
      }
    }
    // A gate whose passing run checks nothing is a failing gate.
    expect(seen).toBeGreaterThan(20);
  });

  it('declares no bare hex and none of §8.7 off-token greys', () => {
    for (const value of allValues()) {
      if (STATED_VERBATIM.has(value)) continue;
      const bare = value.replace(VAR, '');
      expect(bare.match(HEX) ?? []).toEqual([]);
      for (const grey of OFF_TOKEN_GREYS) {
        expect(value.toLowerCase()).not.toContain(grey.toLowerCase());
      }
    }
  });

  it('sets decision-carrying text at --text-3 or lighter and leaves --text-4/--text-5 unused', () => {
    for (const value of allValues()) {
      expect(value).not.toContain('--text-4');
      expect(value).not.toContain('--text-5');
    }
    // §11.3a: the caption is the one thing that qualifies its group, so it is read to decide.
    expect(String(SD.groupCaption.color)).toContain('--text-3');
    // §4.3's caption is the consent boundary, and is raised a rung for it.
    expect(String(SD.privacyCaption.color)).toContain('--text-2');
  });

  it('puts every font size on §8.7 scale and ships every tracked size with its tracking', () => {
    let sized = 0;
    for (const [key, style] of Object.entries(SD)) {
      const record = style as Record<string, unknown>;
      const size = record['fontSize'];
      if (size === undefined) continue;
      sized += 1;
      const px = Number(String(size).replace('px', ''));
      const family = familyOf(record);
      expect(family, `${key} sets a font size and no family`).not.toBeNull();
      expect(isOnScale(family as TypeFamily, px), `${key}: ${String(size)}`).toBe(true);
      // §8.7: mono is always tracked, and so is display at 14px and below — the tracking is
      // what buys the size. `check-type-scale.mjs` enforces the same rule over the file.
      if (family === 'mono' || (family === 'display' && px <= 14)) {
        expect(record['letterSpacing'], `${key} ships without its tracking`).toBeDefined();
      }
    }
    expect(sized).toBeGreaterThan(8);
  });

  it('declares only the durations and the one curve §11.3a states', () => {
    const text = [
      ...(['full', 'reduced', 'off'] as const).flatMap((tier) => [
        String(panelAnimation(tier).animation),
        String(backdropAnimation(tier).animation),
      ]),
      ...allValues(),
    ].join(' ');
    const durations = new Set(text.match(/\b\d+(?:\.\d+)?m?s\b/g) ?? []);
    expect([...durations].sort()).toEqual(['160ms', '200ms', '300ms']);
    const curves = new Set(
      (text.match(/cubic-bezier\([^)]*\)/g) ?? []).map((c) => c.replace(/\s/g, '')),
    );
    expect([...curves]).toEqual(['cubic-bezier(.2,.85,.2,1)']);
  });

  it('animates nothing at off and keeps opacity only at reduced', () => {
    expect(panelAnimation('off').animation).toBe('none');
    expect(backdropAnimation('off').animation).toBe('none');
    // §11.6: reduced drops every transform and clamps what survives to REDUCED_CLAMP_MS.
    expect(String(panelAnimation('reduced').animation)).toBe(`${BACKDROP_KEYFRAME} 160ms both`);
    expect(String(panelAnimation('full').animation)).toContain(PANEL_KEYFRAME);
    expect(String(backdropAnimation('full').animation)).toBe(`${BACKDROP_KEYFRAME} 200ms both`);
  });

  it('animates only keyframes a real stylesheet declares, each exactly once', () => {
    // R36's shape: a name nothing declares animates nothing, and no gate would say so. The
    // `?raw` import returns the empty string when vitest is run from the repository root,
    // which would pass every assertion below vacuously.
    expect(baseCss.length).toBeGreaterThan(0);
    expect(motionCss.length).toBeGreaterThan(0);

    const declared = (css: string): string[] =>
      [...css.matchAll(/@keyframes\s+([A-Za-z_][\w-]*)/g)].map((m) => m[1] as string);
    const base = declared(baseCss);
    const motion = declared(motionCss);

    // R35(b): `viewIn` is shared and is declared in base.css alone, so motion.css's tier clamp
    // can see it. The drawer consumes the name and declares no copy of its own.
    expect(base).toContain(BACKDROP_KEYFRAME);
    expect(motion).not.toContain(BACKDROP_KEYFRAME);

    // The panel's rail slide is this surface's own, so it is declared once, beside the clamps.
    expect(motion.filter((n) => n === PANEL_KEYFRAME)).toHaveLength(1);
    expect(base).not.toContain(PANEL_KEYFRAME);
  });
});
