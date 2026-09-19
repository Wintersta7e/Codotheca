import { type RefObject, useLayoutEffect, useState } from 'react';

/**
 * §8.0a's shed order, the widths it engages at, and the hook that reads a live bar.
 *
 * The table lives here rather than in `TopBar.tsx` because that component imports this module:
 * exporting the hook's own type and table back out of the component would make a runtime import
 * cycle out of what is a leaf table.
 */

export type ShedLevel = 0 | 1 | 2 | 3;

/**
 * §8.0a's stated order: `SWITCH` first — `Alt+Space` still opens quick switch once its chip is
 * gone — then the `SORT` and `DENSITY` keys leaving their values, then the wordmark's lettering
 * leaving its mark. **The order is spec; the widths below are not.**
 */
export const SHED_ORDER = ['switch', 'keys', 'wordmarkLettering'] as const;

/**
 * The bar width at or below which each step engages, in `SHED_ORDER`'s order.
 *
 * **Measured, never asserted** (§8.0a). `app/e2e/topbar-floor.spec.ts` is what produces these: it
 * renders this bar at the widest sort label, in a real layout engine, with the real faces, and
 * fails when a value here is wrong in either direction. Do not adjust one by hand — re-run the
 * measurement.
 *
 * [p3] ~~`[860, 714, 627]`~~ re-measured in the same run that moved `TOP_BAR_FLOOR_PX`: the
 * harness now **derives** its widest label by measuring every `SortKey` variant, and §35.2's
 * `NEEDS ATTENTION` is wider than the `LAST TOUCHED` the harness used to name.
 */
export const SHED_WIDTHS: readonly [number, number, number] = [878, 733, 646];

export const SHED_CLASSES = [
  'cdt-topbar--shed-1',
  'cdt-topbar--shed-2',
  'cdt-topbar--shed-3',
] as const;

export function shedLevelFor(barWidth: number): ShedLevel {
  let level = 0;
  for (const threshold of SHED_WIDTHS) if (barWidth <= threshold) level += 1;
  return level as ShedLevel;
}

/** `null` at rest: a bar that has shed nothing carries no class, rather than a `shed-0` that no
 *  stylesheet declares. */
export function shedClassName(level: ShedLevel): string | null {
  return level === 0 ? null : (SHED_CLASSES[level - 1] ?? null);
}

/**
 * The runtime half: one observer on the bar, and no width constant of its own.
 *
 * State is set only when the level actually changes, so dragging a window across a range that
 * sheds nothing re-renders nothing.
 */
export function useShedLevel(ref: RefObject<HTMLElement | null>): ShedLevel {
  const [level, setLevel] = useState<ShedLevel>(0);

  useLayoutEffect(() => {
    const element = ref.current;
    if (element === null) return undefined;
    const read = (): void => {
      const next = shedLevelFor(element.clientWidth);
      setLevel((current) => (current === next ? current : next));
    };
    read();
    // jsdom has no ResizeObserver. Without the guard a caller under test throws on mount, which
    // reads as a broken component rather than as a missing browser API.
    if (typeof ResizeObserver === 'undefined') return undefined;
    const observer = new ResizeObserver(read);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [ref]);

  return level;
}
