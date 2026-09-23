/**
 * §34.5 — **where the light starts, as a fraction of the scene space.** Never a pixel.
 *
 * One card has three pixel spaces — §7.3's scene document, §7.6's `hero` rendition and §8.5's
 * hero box — and the scaling between them is uniform, so a fraction is exactly right in all three
 * and a pixel figure is right in exactly one. A coordinate its reader must rescale is this
 * project's dominant defect waiting for its second reader.
 *
 * **Derived once, here, from §33's anchor set and nowhere else** — `health.weathering` already
 * reaches the one page the surge plays on, which is why the event carries no origin.
 *
 * **No invented position, ever (A1b).** An empty list, an anchor set that has not arrived and a
 * degenerate space are all *no origin*, and the caller renders the wipe — correct there precisely
 * because such a fix has no point to express. `(0, 0)` is a real corner of the card and is never
 * what absence looks like.
 */
import type { DecayLayer, Point, Weathering } from '../../generated/protocol';

/** Both in `[0, 1]`: a fraction of `spaceW` and of `spaceH`. */
export interface Origin {
  readonly fx: number;
  readonly fy: number;
}

/**
 * The first anchor of `layer`, converted by the kind §33.3 fixes for that layer so no two readers
 * can write two formulas: a point is itself, a polyline its first vertex, a rect its centre.
 */
export function originFor(weathering: Weathering | null, layer: DecayLayer): Origin | null {
  if (weathering === null) return null;
  const { spaceW, spaceH } = weathering;
  if (spaceW <= 0 || spaceH <= 0) return null;
  const anchors = weathering.layers.find((entry) => entry.layer === layer);
  if (anchors === undefined) return null;

  let at: Point | undefined;
  switch (layer) {
    case 'rust':
      at = anchors.points[0];
      break;
    case 'cracks':
      at = anchors.paths[0]?.points[0];
      break;
    case 'dust':
    case 'cobwebs':
    case 'overgrowth': {
      const rect = anchors.rects[0];
      // §34.5's table states the rect's centre as `[x + w/2, y + h/2]`; that halving is the one
      // numeral this derivation carries, and the module that positions the surge carries none.
      at = rect === undefined ? undefined : { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 };
      break;
    }
  }
  if (at === undefined) return null;

  const fx = at.x / spaceW;
  const fy = at.y / spaceH;
  // An anchor outside the space is not a place on the card, and clamping it would invent one.
  if (fx < 0 || fx > 1 || fy < 0 || fy > 1) return null;
  return { fx, fy };
}
