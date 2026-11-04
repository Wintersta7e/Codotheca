import { type RefObject, useEffect, useState } from 'react';

/**
 * Whether the band intersects the shelf's one scroll container — half of §43.9's *shown*, the
 * other half being the window's own focus and visibility (`useWindowActive`).
 *
 * Observed, never polled: an `IntersectionObserver` rooted at the scroll container reports each
 * crossing. A shell without one reports *not on screen*, so a band that cannot be seen to be on
 * screen is never stamped as seen.
 */
export function useBandOnScreen(
  band: RefObject<HTMLElement | null>,
  root: RefObject<HTMLElement | null>,
): boolean {
  const [onScreen, setOnScreen] = useState(false);
  useEffect(() => {
    const target = band.current;
    if (target === null || typeof IntersectionObserver === 'undefined') return undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.target === target) setOnScreen(entry.isIntersecting);
        }
      },
      { root: root.current },
    );
    observer.observe(target);
    return () => {
      observer.disconnect();
    };
  }, [band, root]);
  return onScreen;
}
