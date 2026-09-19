/**
 * Typed mirror of `../styles/tokens.css` (§8.7). The stylesheet is the source; this file is
 * checked against it by `tokens.test.ts`, so the two cannot drift.
 */
export const TOKENS = {
  'app-bg': '#07090b',
  'surface-0': '#0a0d10',
  'surface-1': '#0d1013',
  'surface-2': '#101418',
  'surface-3': '#12161a',
  'surface-4': '#171d23',
  'surface-5': '#1c2228',
  'surface-sel': '#131820',
  'surface-sel-hover': '#141a20',
  'pill-bg': '#1c2a3d',
  'line-1': '#171b20',
  'line-2': '#1f242a',
  'line-3': '#252b32',
  'line-4': '#2c3540',
  'line-5': '#3c454e',
  'sig-edge': '#3a4a5e',
  'edge-strong': '#7f9aa0',
  'text-0': '#f2f5f7',
  'text-1': '#dde3e8',
  'text-2': '#b6c1cb',
  'text-3': '#8b97a3',
  'text-4': '#6c7885',
  'text-5': '#4a5560',
  sig: '#4a9dff',
  'sig-ink': '#08131f',
  'sig-hover': '#8ec2ff',
  pass: '#56704e',
  'pass-ink': '#8fa87c',
  fail: '#8c4a3c',
  'fail-hot': '#e0533d',
  warn: '#c48a4a',
  absent: '#2c3540',
  unknown: '#1e262e',
  'unknown-ink': '#bacede',
  interrupt: '#c8563c',
  silver: '#b9c4cc',
  rust: '#96522a',
  growth: '#54703e',
  // [p3] §33.9's dust tint. A material tint, not a step on §8.7's closed grey ladder.
  dust: '#8e97a0',
  // §8.5.5's lane baseline. An achromatic alpha, so it is not on the ladder and carries no ratio.
  'baseline-hairline': 'rgb(255 255 255 / 0.05)',
  'tier-gold': '#e8c268',
  'tier-brass': '#a8763f',
  'tier-steel': '#5c7c85',
  'tier-plain': '#333c45',
  'tier-blue': '#2f4a5c',
  // §8.7's readable variant of --tier-blue (10.14:1). Reachable from §23.5's blueprint
  // tile: whenever a tier is named in type the readable variant is used, never the frame colour.
  'tier-blue-ink': '#9fc2d6',
  'tier-ref': '#232a31',
  'tier-gold-ink': '#f0d493',
  'tier-brass-ink': '#dca972',
  'tier-steel-ink': '#a3c4cd',
  'tier-plain-ink': '#95a3ae',
  'tier-silver-ink': '#cfd8de',
  'rank-s': 'oklch(0.86 0.13 88)',
  'rank-a': 'oklch(0.74 0.15 300)',
  'rank-b': 'oklch(0.76 0.13 232)',
  'rank-c': 'oklch(0.82 0.02 240)',
  'rank-d': 'oklch(0.78 0.018 240)',
  'rank-e': 'oklch(0.74 0.016 240)',
  'font-display': "'Rajdhani', system-ui, sans-serif",
  'font-body': "'Barlow', system-ui, sans-serif",
  'font-mono': "'JetBrains Mono', ui-monospace, monospace",
  'chamfer-card': '12px',
  'chamfer-hero': '16px',
  'tile-min': '186px',
  'grid-col-gap': '16px',
  'grid-row-gap': '24px',
} as const satisfies Readonly<Record<string, string>>;

export type TokenName = keyof typeof TOKENS;

export function token(name: TokenName): string {
  return `var(--${name})`;
}

export function tokenValue(name: TokenName): string {
  return TOKENS[name];
}

/**
 * The completion ladder in full — gold, archived gold, silver, brass, steel and plain.
 * Plain is a rung, not a neutral: painting it asserts "measured, and under half" (§7.7a).
 * Nothing in phase 1 may paint any of these.
 */
export const LADDER_RUNGS: readonly string[] = [
  '#e8c268',
  '#d9c98f',
  '#b9c4cc',
  '#a8763f',
  '#5c7c85',
  '#333c45',
];

/** The four prototype grounds §8.7 snapped onto existing tokens, plus §8.0b's precedent. */
export const SNAPPED_GROUNDS: readonly string[] = [
  '#0b0e11',
  '#0f1317',
  '#151a20',
  '#141821',
  '#171d24',
];

/**
 * Ten of §8.7's eleven off-token greys, plus the one dropped blue. The ladder is closed.
 *
 * `#cfd6dc` is the eleventh and is **deliberately absent**: §5.4a mandates it as the `archived`
 * condition-dot fill and criterion 58 measures it there, so it is a legal literal in that one
 * role and a gate reading this list must not reject it.
 */
export const OFF_TOKEN_GREYS: readonly string[] = [
  '#f4f8fb',
  '#f4f7f9',
  '#e7ebef',
  '#dfe5ea',
  '#c8d2da',
  '#c3ccd4',
  '#a9b5c0',
  '#9aa6b2',
  '#7a8896',
  '#5f6b76',
  '#6fb0ff',
];
