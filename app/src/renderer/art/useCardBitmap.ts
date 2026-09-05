import { useEffect, useRef, useState } from 'react';
import type { ProjectRow, Rendition, SceneHash } from '../../generated/protocol';
import { artUrl } from '../../shared/artAddress';

export { artUrl };

/**
 * §23.5: which pass a surface asks for. A project with no working copy renders as the line-art
 * drawing of the same seeded machine, and R47 gives that pass its own address per surface —
 * `card-blueprint` on the tile, `hero-blueprint` on the hero — because one variant cannot address
 * two passes over one `scene_hash`.
 */
export function renditionFor(surface: 'card' | 'hero', hasWorkingCopy: boolean): Rendition {
  if (hasWorkingCopy) return surface;
  return surface === 'card' ? 'card-blueprint' : 'hero-blueprint';
}

/**
 * §7.1a's mid-scan flip, closed. A card holds its plate until the bitmap for **that exact**
 * `scene_hash` exists and has decoded, then swaps with **no transition** — no cross-fade, no
 * re-layout, identical geometry. No card changes appearance while the user is watching it.
 *
 * The decode runs on a detached `Image`, which is not a DOM node and which warms the same cache
 * the visible `<img>` then paints from, so the swap costs no second element on a shelf that
 * mounts a hundred and forty of these.
 */
export interface CardBitmapInput {
  readonly sceneHash: SceneHash | null;
  readonly rendition: Rendition;
  readonly artState: ProjectRow['artState'];
  /**
   * The hero's address, from `art.url {rendition:'hero'}` — that request *is* the demand that
   * renders it (plan 10). `''` means there is no address: keep the plate (§7.5).
   */
  readonly src?: string;
  /** Injected so a test can hold a decode open. Production decodes a detached `Image`. */
  readonly decode?: (src: string) => Promise<void>;
}

export interface CardBitmap {
  /** `null` means *render no bitmap*: §7.3a's CSS plate is the ground and the finished fallback. */
  readonly src: string | null;
  /** True while a newer scene hash is decoding behind a bitmap that is still on screen. */
  readonly held: boolean;
}

function decodeImage(src: string): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    const image = new Image();
    image.onload = (): void => {
      resolve();
    };
    image.onerror = (): void => {
      reject(new Error('decode failed'));
    };
    image.src = src;
  });
}

export function useCardBitmap(input: CardBitmapInput): CardBitmap {
  const wanted =
    input.src === undefined
      ? input.artState === 'failed'
        ? null
        : artUrl(input.sceneHash, input.rendition)
      : input.src === ''
        ? null
        : input.src;
  const [shown, setShown] = useState<string | null>(null);

  // The decoder is a seam, not an input: a caller that inlines it would re-run the decode on
  // every render if it were a dependency, and the shelf mounts a hundred and forty of these.
  // The address is the only thing that moves.
  const decodeRef = useRef(input.decode ?? decodeImage);
  useEffect(() => {
    decodeRef.current = input.decode ?? decodeImage;
  });

  useEffect(() => {
    if (wanted === null) {
      setShown(null);
      return undefined;
    }
    let live = true;
    void decodeRef.current(wanted).then(
      () => {
        if (live) setShown(wanted);
      },
      () => {
        /* §7.5: a missing or corrupt file demotes to the nameplate, never a hole. */
      },
    );
    return (): void => {
      live = false;
    };
  }, [wanted]);

  return { src: shown, held: shown !== null && shown !== wanted };
}
