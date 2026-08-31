import { type RefObject, useLayoutEffect, useState } from 'react';

export type ShedLevel = 0 | 1 | 2 | 3;

/**
 * §8.0a's stated order: `SWITCH` first, then the `SORT` and `DENSITY` keys leaving their values,
 * then the wordmark's lettering leaving its mark. The order is spec; the widths are not.
 *
 * **R19**: the type, the table and the threshold function live here rather than in `TopBar.tsx`
 * because that component imports this hook — exporting the hook's own type from the component
 * makes a runtime import cycle out of what is a leaf table.
 */
export const SHED_ORDER = ['switch', 'keys', 'wordmarkLettering'] as const;

/**
 * Measured, never asserted (§8.0a). `app/e2e/topbar-floor.spec.ts` is what sets these against the
 * real bar at the widest sort label; 924px is known to be below the floor. Editing a number here
 * without re-running that spec puts the bar back in the state both shelf captures showed.
 */
export const SHED_WIDTHS: readonly [number, number, number] = [1024, 940, 880];

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

export function shedClassName(level: ShedLevel): string | null {
  return level === 0 ? null : (SHED_CLASSES[level - 1] ?? null);
}

/** The runtime half: one observer on the bar, and no width constant of its own. */
export function useShedLevel(ref: RefObject<HTMLElement | null>): ShedLevel {
  const [level, setLevel] = useState<ShedLevel>(0);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return undefined;
    const read = (): void => {
      const next = shedLevelFor(el.clientWidth);
      setLevel((current) => (current === next ? current : next));
    };
    read();
    const observer = new ResizeObserver(read);
    observer.observe(el);
    return () => {
      observer.disconnect();
    };
  }, [ref]);

  return level;
}
