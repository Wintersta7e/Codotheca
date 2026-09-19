import { describe, expect, it } from 'vitest';
import {
  LADDER_RUNGS,
  OFF_TOKEN_GREYS,
  SNAPPED_GROUNDS,
  TOKENS,
  token,
  tokenValue,
} from './tokens';
// `?raw` avoids node:fs (no renderer Node types) and jsdom's non-file `import.meta.url`.
import tokensCss from '../styles/tokens.css?raw';

// `TOKENS` is `as const` so `TokenName` is a useful union; that closed literal type cannot be
// indexed by a runtime string, and naming an absent key is an error rather than `undefined`.
// Both are exactly what these two assertions need to do, so the test takes a widened view.
const anyToken: Readonly<Record<string, string | undefined>> = TOKENS;

function declarationsInStylesheet(): Map<string, string> {
  const css = tokensCss;
  // A `?raw` import resolves to the EMPTY STRING when vitest runs this file outside the app's
  // own config, and every assertion below then passes against nothing. That has shipped here
  // once — assert the text before reading it.
  expect(css.length, 'tokens.css?raw imported as an empty string').toBeGreaterThan(0);
  const out = new Map<string, string>();
  for (const line of css.split('\n')) {
    const m = /^\s*--([a-z0-9-]+)\s*:\s*([^;]+);/.exec(line);
    if (m?.[1] !== undefined && m[2] !== undefined) out.set(m[1], m[2].trim());
  }
  return out;
}

describe('the token block', () => {
  it('mirrors tokens.css exactly — same names, same values, nothing extra', () => {
    const css = declarationsInStylesheet();
    expect([...css.keys()].sort()).toEqual(Object.keys(TOKENS).sort());
    for (const [name, value] of css) expect(anyToken[name]).toBe(value);
  });

  it('carries the seven tokens v2.2 added', () => {
    expect(tokenValue('surface-sel')).toBe('#131820');
    expect(tokenValue('surface-sel-hover')).toBe('#141a20');
    expect(tokenValue('surface-5')).toBe('#1c2228');
    expect(tokenValue('pill-bg')).toBe('#1c2a3d');
    expect(tokenValue('sig-edge')).toBe('#3a4a5e');
    expect(tokenValue('edge-strong')).toBe('#7f9aa0');
    expect(tokenValue('interrupt')).toBe('#c8563c');
  });

  /**
   * [p3] §33.9's five layer colours, four of which §8.7 already declares.
   *
   * **`--dust` has to exist before a single line of layer CSS is written.** Criterion 46's
   * built-CSS grep fails any undeclared `var(--name)`, and its exemption used to cover *material
   * layers* — so the five literals a layer author would otherwise hard-code were pre-exempted by
   * the bar written to catch them. The exemption is withdrawn in the same change that declares
   * this token (A14.5): §33 states the requirement with its exact name and value and does not
   * edit §8.7.
   *
   * The value is a **material tint, not a step on §8.7's closed grey ladder**, exactly as
   * `--silver` is not — which is why it sits in the *Signal and state* block beside `--rust`,
   * `--growth` and `--silver`.
   *
   * Read off the stylesheet source, not off `TOKENS` alone: a cross-file mirror needs a test
   * reading the other side (R12, R24).
   */
  it('declares the five §33.9 layer tokens in both files', () => {
    const css = declarationsInStylesheet();
    const layers: ReadonlyArray<readonly [string, string]> = [
      ['dust', '#8e97a0'],
      ['silver', '#b9c4cc'],
      ['rust', '#96522a'],
      ['fail', '#8c4a3c'],
      ['growth', '#54703e'],
    ];
    for (const [name, value] of layers) {
      expect(css.get(name), `--${name} is not declared in tokens.css`).toBe(value);
      expect(anyToken[name], `--${name} is missing from the typed mirror`).toBe(value);
    }
    // `--fail-hot` is a different token with a different job and is not a layer colour.
    expect(css.get('fail-hot')).toBe('#e0533d');
  });

  it('carries the derived accent and no deferred alternate', () => {
    expect(tokenValue('sig')).toBe('#4a9dff');
    expect(tokenValue('sig-ink')).toBe('#08131f');
    expect(tokenValue('sig-hover')).toBe('#8ec2ff');
    const values = Object.values(TOKENS).join(' ');
    // Holo cyan, Shadow violet — deferred by §8.7, and a token nothing reads is a dead switch.
    expect(values).not.toContain('#6fd0e8');
    expect(values).not.toContain('#7c5cff');
    // §23.5 lands `Not cloned`, so `--tier-blue-ink` stops being deferred and is asserted as a
    // value rather than as an absence. §8.7 owns it: 10.14:1 against the plate.
    expect(tokenValue('tier-blue-ink')).toBe('#9fc2d6');
    expect(tokenValue('tier-blue')).toBe('#2f4a5c');
  });

  it('declares none of the four snapped grounds and none of the off-token greys', () => {
    const values = Object.values(TOKENS).join(' ');
    for (const literal of SNAPPED_GROUNDS) expect(values).not.toContain(literal);
    for (const literal of OFF_TOKEN_GREYS) expect(values).not.toContain(literal);
  });

  it('names the six completion rungs so no surface can paint one', () => {
    expect(LADDER_RUNGS).toEqual([
      '#e8c268',
      '#d9c98f',
      '#b9c4cc',
      '#a8763f',
      '#5c7c85',
      '#333c45',
    ]);
  });

  it('declares the geometry the grid and the chamfer are built from', () => {
    expect(tokenValue('chamfer-card')).toBe('12px');
    expect(tokenValue('chamfer-hero')).toBe('16px');
    expect(tokenValue('tile-min')).toBe('186px');
    expect(tokenValue('grid-col-gap')).toBe('16px');
    expect(tokenValue('grid-row-gap')).toBe('24px');
  });

  it('renders a token reference, not a literal', () => {
    expect(token('text-3')).toBe('var(--text-3)');
  });
});
