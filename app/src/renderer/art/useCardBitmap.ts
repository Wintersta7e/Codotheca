import { useEffect, useRef, useState } from 'react';
import type { ArtState, ProjectRow, Rendition, SceneHash } from '../../generated/protocol';
import { artUrl } from '../../shared/artAddress';
import { useProjectPageDeps } from '../project/deps';

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
 * §7.6: **the request is the demand.** The core cannot observe a mounted surface, so a rendition
 * that nothing else writes exists only because somebody asked the core for its address — and
 * `art.url` is what rasterises it.
 *
 * Two renditions need this and one does not. J5 writes the `card` rendition during the scan
 * (`core/src/art/job.rs`), so a located tile composes that address and the file is already there.
 * **`hero`, `card-blueprint` and `hero-blueprint` have no other writer at all**: composing their
 * address without asking gets a 404 from the shell and §7.5's plate, permanently. A `null` hash
 * means the caller wants no demand issued, so the hook is still called unconditionally.
 *
 * One implementation, two callers (R12): the hero page and the grid tile. A second copy would be
 * two places for "the request is the demand" to stop being true in.
 */
export function useArtAddress(
  hash: SceneHash | null,
  artState: ArtState,
  rendition: Rendition,
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
        // Whatever is on screen stays there. A rendition that cannot be addressed keeps the last
        // one rather than blanking, and a surface with no bitmap yet keeps the plate.
      });
    return () => {
      live = false;
    };
  }, [deps, hash, artState, rendition]);

  return src;
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
  /**
   * [p3] The hash of the raster **actually on screen**, or `null` when there is none.
   *
   * §33.4 mounts the decay layers only when this equals `Weathering.sceneHash`. It is set in the
   * same state update as `shown`, and **it is not parsed back out of the address**: §7.3a's
   * standing rule is that the renderer resolves nothing from a name, and a URL parser here would
   * be a second resolver in the product.
   */
  readonly decodedSceneHash: SceneHash | null;
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
  // [p3] The decoded address and the scene it belongs to move as one value, so no render can see
  // a hash that does not describe what is painted.
  const [shown, setShown] = useState<{ src: string; hash: SceneHash | null } | null>(null);

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
        if (live) setShown({ src: wanted, hash: input.sceneHash });
      },
      () => {
        /* §7.5: a missing or corrupt file demotes to the nameplate, never a hole. */
      },
    );
    return (): void => {
      live = false;
    };
    // `input.sceneHash` is read inside the resolve rather than depended on: the address is the
    // only thing that moves a decode, and adding the hash would re-run one for a scene whose
    // address is unchanged.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wanted]);

  return {
    src: shown?.src ?? null,
    held: shown !== null && shown.src !== wanted,
    decodedSceneHash: shown?.hash ?? null,
  };
}

/**
 * §24.4's rendition swap: the flip, and the hold.
 *
 * **R47 consumed, not re-declared.** `Rendition`'s four variants, `RENDITIONS` and `renditionFor`
 * are all p2-23's; this hook only decides *when* `hasWorkingCopy` becomes true and what is on
 * screen while the new raster decodes.
 *
 * On `install.finished` the project has a working copy, so `renditionFor` returns `card` on the
 * tile and `hero` on the hero. The swap therefore reads `card-blueprint` → `card` and
 * `hero-blueprint` → `hero` and **never crosses the two** — which is the failure a single
 * `blueprint` variant would have made unavoidable, because one address cannot name two passes.
 *
 * **The hold is §7.1a's.** The decoded blueprint stays on screen until the new rendition's file
 * exists and has decoded, then the swap happens with no transition and identical geometry.
 * `scene_hash` is unchanged and **no art re-render is enqueued**: this is two rasters of one
 * scene, which is why §24.4 calls it a rendition swap and not a re-render.
 *
 * The scene is a parameter rather than module state: two tiles can be mid-flip at once, and a
 * shared mutable scene would let one clobber the other's address.
 */
export function useInstalledRenditionFlip(
  surface: 'card' | 'hero',
  sceneHash: SceneHash | null,
  hasWorkingCopy: boolean,
  installFinished: boolean,
): Rendition {
  const target = renditionFor(surface, hasWorkingCopy || installFinished);
  const [shown, setShown] = useState<Rendition>(target);

  useEffect(() => {
    if (target === shown) return undefined;
    // No scene means no address to preload — `artUrl` answers `null` for an absent or empty
    // hash. The blueprint stays up rather than being swapped for a plate: an old raster of the
    // right scene beats an empty frame.
    const address = artUrl(sceneHash, target);
    if (address === null) return undefined;
    let live = true;
    // The blueprint is held until the new rendition has actually decoded. Swapping on the event
    // alone would blank the tile for as long as the raster takes, at exactly the moment the user
    // is watching it.
    const image = new Image();
    image.onload = () => {
      if (live) setShown(target);
    };
    // A rendition that never decodes leaves the blueprint up, which is the honest outcome: the
    // old raster is a real picture of the same scene, and an empty plate is not.
    image.onerror = () => {};
    image.src = address;
    return () => {
      live = false;
    };
  }, [target, shown, sceneHash]);

  return shown;
}
