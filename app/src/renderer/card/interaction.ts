import { type CardAppearance, jewelAlpha } from '../art/appearance';
import type { ResolvedTier } from '../motion/tier';

/**
 * §7.8's three interaction states. Hover is component state — `onMouseEnter`/`onMouseLeave` set
 * `hov = <projectId>` and everything here derives from it — never CSS `:hover`: six of the ten
 * effects live on descendants and a seventh shares the card's own `box-shadow` with the
 * selection ring, so one value must be legible to every layer at once.
 */
export type HoverEffectId =
  | 'lift'
  | 'frame'
  | 'ledRun'
  | 'bloom'
  | 'specular'
  | 'brackets'
  | 'scanLine'
  | 'sigil'
  | 'conditionDot'
  | 'dataStrip';

export interface HoverEffect {
  readonly id: HoverEffectId;
  readonly element: string;
  /**
   * `transform` is dropped below `full`; `travelling` is the LED run, the specular sweep, the
   * brackets and the scan line, which are absent below `full`; `state` renders at every tier and
   * only its transition is clamped.
   */
  readonly kind: 'transform' | 'travelling' | 'state';
}

export const HOVER_EFFECTS: readonly HoverEffect[] = [
  { id: 'lift', element: '.cdt-card', kind: 'transform' },
  { id: 'frame', element: '.cdt-card', kind: 'state' },
  { id: 'ledRun', element: '.cdt-card', kind: 'travelling' },
  // Outer shadows cannot live on the clipped card: `clip-path` deletes them exactly as it
  // deletes `outline`. Plan 12 puts this one on an unclipped sibling.
  { id: 'bloom', element: '.cdt-bloom', kind: 'state' },
  { id: 'specular', element: '.cdt-plate', kind: 'travelling' },
  { id: 'brackets', element: '.cdt-bracket', kind: 'travelling' },
  { id: 'scanLine', element: '.cdt-scanline', kind: 'travelling' },
  { id: 'sigil', element: '.cdt-sigil', kind: 'transform' },
  { id: 'conditionDot', element: '.cdt-dot', kind: 'transform' },
  { id: 'dataStrip', element: '.cdt-strip', kind: 'state' },
];

export function effectsAt(tier: ResolvedTier): readonly HoverEffectId[] {
  if (tier === 'full') return HOVER_EFFECTS.map((effect) => effect.id);
  return HOVER_EFFECTS.filter((effect) => effect.kind === 'state').map((effect) => effect.id);
}

export const LIFT_TRANSFORM = 'translateY(-7px) scale(1.025)';
export const PRESS_TRANSFORM = 'translateY(-3px) scale(.986)';
/** §5.4a's hover scale; §11.6 owns its duration and curve and this module restates neither. */
export const DOT_HOVER_TRANSFORM = 'scale(1.5)';
export const BRACKET_STAGGER_S: readonly ['0s', '.03s', '.06s', '.09s'] = [
  '0s',
  '.03s',
  '.06s',
  '.09s',
];

/**
 * The one focus indicator, and the selection indicator, which are the same thing. It is an
 * **inset** ring in the element's own `box-shadow` — never an `outline`, which the chamfer clips
 * away entirely, and never an overlay layer, one of which silently failed to mount in review.
 *
 * The hard ring is `jewelInk` and is fade-independent; only the soft inset bloom keeps the faded
 * jewel. Measured against the lightest tone a plate emits, a jewel ring runs 2.65–4.79 : 1 over
 * the eight hue bins and both fades — legible on most cards and under the 3 : 1 non-text floor on
 * some, with nothing on screen to say which, and worst on exactly the faded reference and
 * archived cards a triage pass arrows through. `jewelInk` is 11.1–11.7 : 1 on every one of them.
 * It never animates, so no motion tier removes it.
 */
export function selectionRing(appearance: CardAppearance): string {
  return [
    `inset 0 0 0 2px ${appearance.jewelInk}`,
    `inset 0 0 22px -6px ${jewelAlpha(appearance, 0.55)}`,
  ].join(', ');
}

export function bloomShadow(appearance: CardAppearance): string {
  return `0 22px 34px -16px rgb(0 0 0 / .8), 0 0 32px -8px ${jewelAlpha(appearance, 0.85)}`;
}

export function ledGradient(appearance: CardAppearance): string {
  return (
    `linear-gradient(90deg, transparent 0 6%, ${appearance.jewel} 16%, #ffffff 23%, ` +
    `${appearance.jewel} 30%, transparent 42%)`
  );
}

export function scanGradient(appearance: CardAppearance): string {
  return (
    'linear-gradient(180deg, transparent, rgb(255 255 255 / .18) 68%, ' +
    `${jewelAlpha(appearance, 0.8)} 96%, transparent)`
  );
}

/**
 * The only channel by which §7.3a's palette reaches the DOM. It sets **values**, never
 * properties: an inline `background-image` would outrank
 * `[data-effects-tier='off'] .cdt-card { background-image: none }` — inline style beats any
 * selector — and the LED run would keep travelling at tier `off`. Every declaration that reads
 * one of these lives in `card.css`, where plan 12's stylesheet gate can see it and where the
 * `--cdt-` prefix is what makes it legal.
 */
export function cardCustomProperties(
  appearance: CardAppearance,
  frameColour: string,
): Record<string, string> {
  return {
    '--cdt-frame': frameColour,
    '--cdt-jewel': appearance.jewel,
    '--cdt-jewel-ink': appearance.jewelInk,
    '--cdt-jewel-85': jewelAlpha(appearance, 0.85),
    '--cdt-jewel-80': jewelAlpha(appearance, 0.8),
    '--cdt-jewel-55': jewelAlpha(appearance, 0.55),
    '--cdt-jewel-30': jewelAlpha(appearance, 0.3),
    '--cdt-jewel-12': jewelAlpha(appearance, 0.12),
    '--cdt-plate': appearance.plate,
    '--cdt-greebling': appearance.greebling,
    '--cdt-led': ledGradient(appearance),
    '--cdt-scan': scanGradient(appearance),
  };
}
