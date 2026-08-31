import type { ProjectRow } from '../../generated/protocol';
import { jewelInkFor } from '../theme/contrast';

/**
 * §7.3a's derivation, in the renderer, because §7.7's language plate border, §7.7's jewel stripe,
 * §7.8's ten hover layers and §11.7's focus ring are all DOM and all lit by it.
 *
 * The hash runs over the **UTF-16 code units of `seed_basename`** — the directory basename
 * recorded at first index — and never over `project.name`, which absorbs the remote name and
 * would re-roll a clone in a differently-named folder the moment the remote is learned.
 */
export interface ArtSeed {
  readonly seedBasename: string;
  readonly rerollOffset: number;
}

export interface CardAppearance {
  readonly hue: number;
  readonly jewel: string;
  readonly jewelInk: string;
  readonly plate: string;
  readonly greebling: string;
  readonly panelFamily: number;
  readonly liveryFamily: number;
  readonly designation: string;
  readonly fade: number;
}

export const JEWEL_HUES: readonly number[] = [26, 58, 96, 148, 188, 232, 284, 328];

export const LANG_CODES: Readonly<Record<string, string>> = {
  Rust: 'RS',
  TypeScript: 'TS',
  Python: 'PY',
  'C++': 'CP',
  'C#': 'CS',
  JavaScript: 'JS',
  Java: 'JV',
  Go: 'GO',
  Shell: 'SH',
  Lua: 'LU',
};

const ROMAN = ['I', 'II', 'III', 'IV', 'V', 'VI', 'VII', 'VIII', 'IX', 'X'] as const;

/** The design notes label this set "chosen by hash % 4"; that is the *livery* selector. */
export const GREEBLING_FAMILIES: readonly [string, string, string, string] = [
  'repeating-linear-gradient(0deg, rgb(0 0 0 / .13) 0 1px, transparent 1px 20px), repeating-linear-gradient(90deg, rgb(0 0 0 / .1) 0 1px, transparent 1px 32px)',
  'repeating-linear-gradient(90deg, rgb(0 0 0 / .15) 0 2px, transparent 2px 38px), repeating-linear-gradient(0deg, rgb(255 255 255 / .035) 0 1px, transparent 1px 28px)',
  'repeating-linear-gradient(0deg, rgb(0 0 0 / .12) 0 1px, transparent 1px 15px), radial-gradient(circle at 76% 20%, rgb(255 255 255 / .055), transparent 40%)',
  'repeating-linear-gradient(90deg, rgb(0 0 0 / .13) 0 1px, transparent 1px 24px), repeating-linear-gradient(0deg, rgb(0 0 0 / .09) 0 2px, transparent 2px 42px)',
];

export function hashSeed(seed: ArtSeed): number {
  const s =
    seed.rerollOffset === 0
      ? seed.seedBasename
      : `${seed.seedBasename}#${String(seed.rerollOffset)}`;
  let h = 0;
  for (let i = 0; i < s.length; i += 1) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return h;
}

export function seedOf(row: Pick<ProjectRow, 'seedBasename' | 'rerollOffset'>): ArtSeed {
  return { seedBasename: row.seedBasename, rerollOffset: row.rerollOffset };
}

/**
 * §7.3a owns this value and pins it: two flat cases and a zero, **never a function of elapsed
 * time**. A fade that moves with the clock makes the scene *address* a function of wall-clock
 * time — every card re-hashes and re-rasterizes as the days pass, the disk cache grows without
 * bound, and the recognition §7.4 defends is lost to arithmetic. Phase 3 changes this one line.
 */
export function fadeFor(row: Pick<ProjectRow, 'isReference' | 'isArchived'>): number {
  return row.isReference || row.isArchived ? 0.25 : 0;
}

export function languageCode(primaryLanguage: string): string {
  return LANG_CODES[primaryLanguage] ?? 'GN';
}

/** Four decimals: enough to keep `c × 1.06` and `c × 0.8` distinct, few enough to be stable. */
const r = (value: number): string => String(Math.round(value * 1e4) / 1e4);

const oklch = (l: number, c: number, hue: number): string =>
  `oklch(${r(l)} ${r(c)} ${String(hue)})`;

export function appearanceFor(
  seed: ArtSeed,
  fade: number,
  primaryLanguage: string | null,
): CardAppearance {
  const h = hashSeed(seed);

  const bin = JEWEL_HUES[h % 8] ?? 26;
  const hue = bin + (((h >>> 5) % 7) - 3);
  const jewelL = 0.6 - ((h >>> 9) % 3) * 0.035 - fade * 0.12;
  const jewelC = (0.175 - ((h >>> 13) % 3) * 0.02) * (1 - fade * 0.5);

  const plateHue = 76 + (((h >>> 5) % 9) - 4);
  const c = (0.005 + ((h >>> 9) % 3) * 0.002) * (1 - fade * 0.5);
  const step = ((h >>> 21) % 5) * 0.011;
  const hi = 0.185 + step - fade * 0.02;
  const mid = 0.145 + step - fade * 0.016;
  const lo = 0.085 + step * 0.5 + fade * 0.005;
  const ang = [148, 32, 118, 62][(h >>> 13) % 4] ?? 148;
  const split = 38 + ((h >>> 17) % 24);

  const panelFamily = (h >>> 3) % 4;
  const liveryFamily = h % 4;
  const v = (n: number): number => (h >>> n) % 100;
  const prefix = primaryLanguage === null ? 'GN' : languageCode(primaryLanguage);

  return {
    hue,
    jewel: oklch(jewelL, jewelC, hue),
    jewelInk: jewelInkFor(hue),
    plate:
      `linear-gradient(${String(ang)}deg, ${oklch(hi, c, plateHue)} 0 ${String(split)}%, ` +
      `${oklch(mid, c * 1.06, plateHue)} ${String(split)}% 100%), ` +
      `linear-gradient(157deg, ${oklch(hi, c, plateHue)}, ${oklch(lo, c * 0.8, plateHue)})`,
    greebling: GREEBLING_FAMILIES[panelFamily] ?? GREEBLING_FAMILIES[0],
    panelFamily,
    liveryFamily,
    designation: `${prefix}-${String(10 + (v(11) % 89))} / MK-${ROMAN[v(3) % 10] ?? 'I'}`,
    // Recorded so no layer re-derives it: §7.8's hover light is at the fade the scene document
    // holds, which is this value and never a clock read at hover time.
    fade,
  };
}

/** `<jewel .55>`, `<jewel .8>`, `<jewel .85>` — the same colour, an alpha away. */
export function jewelAlpha(appearance: CardAppearance, alpha: number): string {
  const suffix = String(alpha).replace(/^0/, '');
  return appearance.jewel.replace(/\)$/, ` / ${suffix})`);
}
