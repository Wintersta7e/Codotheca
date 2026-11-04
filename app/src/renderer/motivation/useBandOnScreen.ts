import { type RefObject, useEffect, useState } from 'react';

/**
 * Whether the band intersects the shelf's one scroll container — half of §43.9's *shown*, the
 * other half being the window's own focus and visibility (`useWindowActive`).
 *
 * Observed, never polled: an `IntersectionObserver` rooted at the scroll container reports each
 * crossing. A shell without one reports *not on screen*, so a band that cannot be seen to be on
 * screen is never stamped as seen.
 *
 * `report` hands each observed crossing out, for §43.2's condensed block, and `null` once the band
 * is no longer observed: before the first crossing is reported the band's place is not known,
 * and the block is not drawn on a guess.
 */
export function useBandOnScreen(
  band: RefObject<HTMLElement | null>,
  root: RefObject<HTMLElement | null>,
  report?: (onScreen: boolean | null) => void,
): boolean {
  const [onScreen, setOnScreen] = useState(false);
  useEffect(() => {
    const target = band.current;
    if (target === null || typeof IntersectionObserver === 'undefined') return undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.target !== target) continue;
          setOnScreen(entry.isIntersecting);
          report?.(entry.isIntersecting);
        }
      },
      { root: root.current },
    );
    observer.observe(target);
    return () => {
      observer.disconnect();
      report?.(null);
    };
  }, [band, root, report]);
  return onScreen;
}
