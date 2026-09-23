/**
 * Types for `motion-clamp.mjs`, so `app/test/tierClamp.test.ts` reads the tier clamp through the
 * same parser `scripts/check-motion-clamp.mjs` does rather than a second one that can drift from
 * it.
 */
export interface ClampSets {
  readonly displayNone: ReadonlySet<string>;
  readonly noTransform: ReadonlySet<string>;
  readonly clamped: ReadonlySet<string>;
  readonly all: ReadonlySet<string>;
}

export interface CssRule {
  readonly selectors: readonly string[];
  readonly body: string;
  readonly context: readonly string[];
}

export declare function withoutCssComments(css: string): string;
export declare function cssRules(css: string): CssRule[];
export declare function declarations(body: string): { property: string; value: string }[];
export declare function durationsMs(value: string): number[];
export declare function transitionDurationsMs(value: string): number[];
export declare function tierOf(selector: string): string | null;
export declare function subjectClasses(selector: string): string[];
export declare function clampClassNames(css: string): ClampSets;

/** `[ids, classes, types]`. */
export type Specificity = readonly [number, number, number];
export type MotionFamily = 'transform' | 'animation' | 'transition';

export declare const UNWEIGHED_PSEUDO: RegExp;
export declare function specificity(selector: string): Specificity;
export declare function compareSpecificity(a: Specificity, b: Specificity): number;
export declare function motionFamilies(body: string, clampMs: number): Set<MotionFamily>;
export declare function clampFamilies(
  body: string,
  tier: string,
  clampMs: number,
): Set<MotionFamily | 'all'>;
