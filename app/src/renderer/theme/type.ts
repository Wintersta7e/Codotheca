/**
 * §8.7's type scale, completed. The list published in the earlier round was transcribed from
 * the handoff README, which omitted twelve display sizes this document then specified
 * normatively — a build enforcing that list would reject the spec's own surfaces.
 *
 * 7px is the floor. Nothing renders below it: the prototype breached it three times at 6.5px
 * and all three go to 7px with tracking unchanged.
 */
export type TypeFamily = 'display' | 'body' | 'mono';

export const TYPE_FLOOR_PX = 7;

export const DISPLAY_SIZES: readonly number[] = [
  84, 64, 52, 46, 44, 42, 40, 38, 36, 34, 32, 30, 27, 26, 22, 21, 20, 19, 17, 16, 15, 14, 13.5, 13,
  12.5, 12, 10,
];

export const BODY_SIZES: readonly number[] = [14, 13.5, 13, 12.5, 12, 11.5, 11, 10];

export const MONO_SIZES: readonly number[] = [13, 11.5, 11, 10.5, 10, 9.5, 9, 8.5, 8, 7.5, 7];

export function sizesFor(family: TypeFamily): readonly number[] {
  if (family === 'display') return DISPLAY_SIZES;
  if (family === 'body') return BODY_SIZES;
  return MONO_SIZES;
}

export function isOnScale(family: TypeFamily, px: number): boolean {
  return sizesFor(family).includes(px);
}

/**
 * Letter-spacing is load-bearing at these sizes: a mono label at 7.5px without its tracking is
 * a smear, not small type. A size and its tracking ship together or neither ships.
 */
export const TRACKING_RANGE: Readonly<Record<TypeFamily, readonly [number, number] | null>> = {
  display: [0.1, 0.24],
  body: null,
  mono: [0.1, 0.36],
};

export function needsTracking(family: TypeFamily, px: number): boolean {
  if (family === 'mono') return true;
  if (family === 'display') return px <= 14;
  return false;
}
