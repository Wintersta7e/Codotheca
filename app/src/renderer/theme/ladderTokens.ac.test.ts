import { describe, expect, it } from 'vitest';
import tokensCss from '../styles/tokens.css?raw';
import { RUNG_TOKENS, TOKENS, type TokenName, tokenValue } from './tokens';

/**
 * **`AC-P3-31-8` — a rung is a token, never a hex.**
 *
 * Phase 3 is the first phase that **paints** a rung, so this is the first time the guard's set
 * and the stylesheet's declarations have to agree. Three of the six had no declaration at all
 * before §31.1c: silver would have resolved to `--silver`, which §33 reads for cobwebs, and
 * archived gold had neither a frame nor an ink.
 *
 * **The rung set is derived from the stylesheet and printed**; a run that resolves zero rungs
 * fails. **Colours are compared as numbers, never as strings** — the Write/Edit auto-format hook
 * rewrites `.css` and `fmt:check` does not cover it, so `#d9c98f` and `#D9C98F` and any
 * whitespace the formatter chooses must all compare equal.
 */

/** Every custom property the stylesheet declares, as a map. */
function declarations(css: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const match of css.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/giu)) {
    const name = match[1];
    const value = match[2];
    if (name !== undefined && value !== undefined) out.set(name.trim(), value.trim());
  }
  return out;
}

/** `#d9c98f` → `0xd9c98f`. A number, so case and spacing cannot make two equal values differ. */
function rgbNumber(value: string): number | null {
  const hex = /^#([0-9a-f]{6})$/iu.exec(value.trim());
  if (hex?.[1] === undefined) return null;
  return Number.parseInt(hex[1], 16);
}

describe('AC-P3-31-8: every rung the renderer resolves is a declared custom property', () => {
  it('reads a non-empty stylesheet before asserting anything about it', () => {
    // Nine of ten assertions below would pass vacuously against an empty string.
    expect(tokensCss.length).toBeGreaterThan(0);
    expect(tokensCss).toContain('--tier-gold');
  });

  it('declares every rung frame and every rung ink, derived from the stylesheet', () => {
    const declared = declarations(tokensCss);
    const resolved: string[] = [];
    const inks: TokenName[] = [
      'tier-gold-ink',
      'tier-gold-archived-ink',
      'tier-silver-ink',
      'tier-brass-ink',
      'tier-steel-ink',
      'tier-plain-ink',
    ];

    for (const name of [...RUNG_TOKENS, ...inks]) {
      const css = declared.get(`--${name}`);
      expect(css, `--${name} is not declared in §8.7's block`).toBeDefined();
      const fromCss = rgbNumber(css ?? '');
      const fromMap = rgbNumber(tokenValue(name));
      expect(fromCss, `--${name} is not a six-digit hex: ${String(css)}`).not.toBeNull();
      // Numbers, not strings.
      expect(fromCss).toBe(fromMap);
      resolved.push(name);
    }

    console.error(
      `ladderTokens: resolved ${String(resolved.length)} rung token(s): ${resolved.join(', ')}`,
    );
    expect(resolved.length, 'a run that resolved zero rungs proves nothing').toBeGreaterThan(0);
    expect(resolved).toHaveLength(12);
  });

  it('keeps the silver rung off --silver, which another subsystem reads', () => {
    const declared = declarations(tokensCss);
    // The two hold the same value today and are two declarations on purpose: §33 uses `--silver`
    // for cobwebs, and one value two unrelated subsystems share is one value that drifts.
    expect(declared.get('--tier-silver')).toBeDefined();
    expect(declared.get('--silver')).toBeDefined();
    expect(RUNG_TOKENS).not.toContain('silver');
  });

  it('holds archived gold and its ink, whose provenance is the prototype alone', () => {
    expect(rgbNumber(TOKENS['tier-gold-archived'])).toBe(0xd9_c9_8f);
    expect(rgbNumber(TOKENS['tier-gold-archived-ink'])).toBe(0xe6_da_b4);
    // The ink is NOT a rung: it is drawn in type, never as the frame.
    expect(RUNG_TOKENS).not.toContain('tier-gold-archived-ink');
  });
});
