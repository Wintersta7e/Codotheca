/**
 * §8.0a's shed order and the widths at which each step engages.
 *
 * These four names are a leaf table. `TopBar.tsx` imports them, so declaring them in that
 * component and exporting them from it would make an import cycle out of a constant list.
 *
 * The hook that reads the live bar width and turns it into a level belongs in this module too
 * and is not written here — it observes the element, which is the grid lane's concern. Nothing
 * in this file has a React dependency, so adding the hook beside it changes none of it.
 */

/** §8.0a's stated order. `Alt+Space` still opens quick switch once its chip is gone. */
export const SHED_ORDER = ['switch', 'keys', 'wordmarkLettering'] as const;

/**
 * The bar width at or below which each step engages, in `SHED_ORDER`'s order.
 *
 * **Provisional.** §8.0a's clause is that the floor is *measured, not asserted*, and
 * `app/e2e/topbar-floor.spec.ts` is what measures it — these three are replaced by that run's
 * output, not adjusted by hand.
 */
export const SHED_WIDTHS = [1024, 976, 948] as const;

export type ShedLevel = 0 | 1 | 2 | 3;

export function shedLevelFor(barWidth: number): ShedLevel {
  let level = 0;
  for (const width of SHED_WIDTHS) if (barWidth <= width) level += 1;
  return level as ShedLevel;
}
