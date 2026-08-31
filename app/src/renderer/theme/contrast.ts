/**
 * Contrast arithmetic for §8.7's floors.
 *
 * §8.7's floors were corrected across two review rounds; this file computes them rather than
 * quoting them, so a token edit that lowers a ratio fails a test instead of shipping. The
 * floor is a **number** — 5.9:1 — measured against the nearest painted ancestor, never the
 * token `--text-3`, whose own ratio moves with the ground under it.
 */
export interface Rgb {
  readonly r: number;
  readonly g: number;
  readonly b: number;
}

export const DECISION_FLOOR = 5.9;
export const NON_TEXT_FLOOR = 3;

/**
 * The lightest tone any plate can emit: `hi = 0.185 + 4 × 0.011` at `c = 0.005 + 2 × 0.002`,
 * which is the worst case only because `fade` is pinned to 0 in phase 1 (§7.3a).
 */
export const LIGHTEST_PLATE_STOP = { l: 0.229, c: 0.009, hue: 76 } as const;

export function parseHex(hex: string): Rgb {
  const body = hex.trim().replace('#', '');
  if (!/^[0-9a-fA-F]{6}$/.test(body)) throw new Error(`not a 6-digit hex colour: ${hex}`);
  return {
    r: Number.parseInt(body.slice(0, 2), 16),
    g: Number.parseInt(body.slice(2, 4), 16),
    b: Number.parseInt(body.slice(4, 6), 16),
  };
}

const clamp01 = (v: number): number => Math.min(1, Math.max(0, v));

/**
 * OKLCH → OKLab → linear sRGB → gamma-encoded sRGB, clipped into gamut.
 *
 * Keep channels at full precision here: 8-bit quantization is a rasterization step, not a
 * colour-space step. Quantizing before luminance moves §8.7's published ratios by up to 0.06,
 * enough to cross a two-decimal assertion.
 */
export function oklchToRgb(l: number, c: number, hueDeg: number): Rgb {
  const h = (hueDeg * Math.PI) / 180;
  const a = c * Math.cos(h);
  const bb = c * Math.sin(h);
  const lp = (l + 0.3963377774 * a + 0.2158037573 * bb) ** 3;
  const mp = (l - 0.1055613458 * a - 0.0638541728 * bb) ** 3;
  const sp = (l - 0.0894841775 * a - 1.291485548 * bb) ** 3;
  const linear = [
    4.0767416621 * lp - 3.3077115913 * mp + 0.2309699292 * sp,
    -1.2684380046 * lp + 2.6097574011 * mp - 0.3413193965 * sp,
    -0.0041960863 * lp - 0.7034186147 * mp + 1.707614701 * sp,
  ].map((v) => {
    const x = clamp01(v);
    return (x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055) * 255;
  });
  return { r: linear[0] ?? 0, g: linear[1] ?? 0, b: linear[2] ?? 0 };
}

const channel = (eight: number): number => {
  const v = eight / 255;
  return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
};

export function relativeLuminance(rgb: Rgb): number {
  return 0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b);
}

export function contrastRatio(a: Rgb, b: Rgb): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

const OKLCH = /^oklch\(\s*([\d.]+)\s+([\d.]+)\s+(-?[\d.]+)\s*\)$/;

function toRgb(colour: string | Rgb): Rgb {
  if (typeof colour !== 'string') return colour;
  const m = OKLCH.exec(colour.trim());
  if (m?.[1] !== undefined && m[2] !== undefined && m[3] !== undefined) {
    return oklchToRgb(Number(m[1]), Number(m[2]), Number(m[3]));
  }
  return parseHex(colour);
}

export function ratioOf(foreground: string | Rgb, background: string | Rgb): number {
  return contrastRatio(toRgb(foreground), toRgb(background));
}

/** Source-over composite of a translucent fill on an opaque ground, in 8-bit sRGB. */
export function compositeOver(fg: Rgb, alpha: number, bg: Rgb): Rgb {
  const mix = (f: number, b: number): number => Math.round(alpha * f + (1 - alpha) * b);
  return { r: mix(fg.r, bg.r), g: mix(fg.g, bg.g), b: mix(fg.b, bg.b) };
}

export function meetsDecisionFloor(foreground: string | Rgb, background: string | Rgb): boolean {
  return ratioOf(foreground, background) >= DECISION_FLOOR;
}

export function meetsNonTextFloor(foreground: string | Rgb, background: string | Rgb): boolean {
  return ratioOf(foreground, background) >= NON_TEXT_FLOOR;
}

/** The fade-independent readable variant of a jewel (§7.3a, §7.8). */
export function jewelInkFor(hueDeg: number): string {
  return `oklch(0.87 0.07 ${String(hueDeg)})`;
}
