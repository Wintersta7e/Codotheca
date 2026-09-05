/**
 * §7.6: the renderer asks the core for the hero's address rather than composing one, and that
 * request **is** the demand that renders it — the core cannot observe an open page, so a hero
 * rendition exists only because somebody asked for its address.
 *
 * §7.1a's hold-and-swap lives one level down, in the bitmap hook every surface shares. This hook
 * feeds it, so it holds the previous address until the next one has been answered: clearing to
 * `''` in between would blank the bitmap and flash the plate, which is the exact thing the swap
 * exists to prevent.
 */
import { useEffect, useState } from 'react';

import type { ArtState, Rendition, SceneHash } from '../../../generated/protocol';
import { useProjectPageDeps } from '../deps';

/** `''` means *no address*: §7.5's plate stands. */
export function useHeroArt(
  hash: SceneHash | null,
  artState: ArtState,
  rendition: Rendition = 'hero',
): string {
  const deps = useProjectPageDeps();
  const [src, setSrc] = useState('');

  useEffect(() => {
    // A rendition the core has already failed on is not worth asking for; the plate is the
    // finished fallback, not a degraded one.
    if (hash === null || artState === 'failed') return undefined;
    let live = true;
    deps
      .request('art.url', { hash, rendition })
      .then((url) => {
        if (live && url !== '') setSrc(url);
      })
      .catch(() => {
        // Whatever is on screen stays there. A hero that cannot be addressed keeps the last one
        // rather than blanking, and a project with no bitmap yet keeps the plate.
      });
    return () => {
      live = false;
    };
  }, [deps, hash, artState, rendition]);

  return src;
}
