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
import type { ArtState, Rendition, SceneHash } from '../../../generated/protocol';
import { useArtAddress } from '../../art/useCardBitmap';

/**
 * `''` means *no address*: §7.5's plate stands.
 *
 * The body moved to `useArtAddress`, which the grid tile needs too: a `card-blueprint` has no
 * writer but this request either, and a second copy of the rule would be a second place for it
 * to stop being true.
 */
export function useHeroArt(
  hash: SceneHash | null,
  artState: ArtState,
  rendition: Rendition = 'hero',
): string {
  return useArtAddress(hash, artState, rendition);
}
