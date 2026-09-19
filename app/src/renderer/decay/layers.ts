/**
 * §33.4's geometry: one resolved anchor set, converted to percentages, positioned over the plate.
 *
 * **Positions are percentages.** One anchor set serves both renditions because §7.3's space and
 * both of §7.6's renditions are the same ratio — divide by `spaceW` / `spaceH`. **A rendition at
 * any other ratio breaks this** and must carry a second anchor set or letterbox; stated here
 * rather than discovered by whoever adds one.
 *
 * **The renderer never parses `scene_json`.** It receives resolved numbers from the core and
 * converts them.
 */
import type { DecayLayer, WeatherLayer } from '../../generated/protocol';

/**
 * The deepest stop in the prototype's damage ramp, and **the one owner of the ceiling**.
 * No layer's composited alpha over any pixel exceeds it, which bounds what decay can do to the
 * art and forbids it doing anything at all to type — the stack sits below all five bands.
 *
 * `app/test/decayCss.test.ts` reads this value rather than restating it.
 */
export const DECAY_ALPHA_MAX = 0.62;

/** The generated enum's five variants in declaration order — the order anchors light in. */
export const DECAY_LAYER_ORDER: readonly DecayLayer[] = [
  'dust',
  'cobwebs',
  'rust',
  'cracks',
  'overgrowth',
];

/** A sprite's box over the plate, in percentages. **Never a pixel.** */
export interface AnchorBox {
  readonly leftPct: number;
  readonly topPct: number;
  readonly widthPct: number;
  readonly heightPct: number;
}

function pct(value: number, span: number): number {
  return (value / span) * 100;
}

/**
 * §33.4's attachment table, as geometry.
 *
 * | Layer | Attachment |
 * |---|---|
 * | `dust` | settles along the anchor rect's **top edge** |
 * | `cobwebs` | spans from the anchor rect's top-right corner to the card's top-right corner |
 * | `rust` | a stain originating at the point and running **+y**, because the space declares `yDown` |
 * | `cracks` | an offset from each segment of the polyline |
 * | `overgrowth` | emerges from the anchor rect's **bottom edge** and grows **−y** |
 *
 * Only `cobwebs` needs geometry the anchor does not already carry — its box is the span between
 * two corners — which is why the table lives here rather than entirely in `decay.css`. The other
 * four take the anchor's own box and `decay.css` decides which edge the paint hangs from.
 *
 * A layer with no anchors converts to no boxes. **No `(0,0)` fallback and no synthetic rect**
 * (A1b): an empty anchor list is never repaired by an invented coordinate.
 */
export function anchorPercents(
  layer: WeatherLayer,
  spaceW: number,
  spaceH: number,
): readonly AnchorBox[] {
  if (spaceW <= 0 || spaceH <= 0) return [];
  switch (layer.layer) {
    case 'cobwebs':
      // From the module's top-right corner to the card's, which is (spaceW, 0).
      return layer.rects.map((r) => ({
        leftPct: pct(r.x + r.w, spaceW),
        topPct: 0,
        widthPct: pct(spaceW - (r.x + r.w), spaceW),
        heightPct: pct(r.y, spaceH),
      }));
    case 'rust':
      // The point alone. `decay.css` gives the stain its size and runs it +y.
      return layer.points.map((p) => ({
        leftPct: pct(p.x, spaceW),
        topPct: pct(p.y, spaceH),
        widthPct: 0,
        heightPct: 0,
      }));
    case 'cracks':
      // Each polyline's own extent. A seam of one point has no extent and takes a zero box,
      // which `decay.css` draws as a hairline rather than as nothing.
      return layer.paths.map((path) => {
        // A polyline with no point has no extent. Seeding the min and max with `0` instead would
        // stretch every crack back to the card's origin, which is an invented coordinate.
        if (path.points.length === 0) {
          return { leftPct: 0, topPct: 0, widthPct: 0, heightPct: 0 };
        }
        const xs = path.points.map((p) => p.x);
        const ys = path.points.map((p) => p.y);
        const minX = Math.min(...xs);
        const maxX = Math.max(...xs);
        const minY = Math.min(...ys);
        const maxY = Math.max(...ys);
        return {
          leftPct: pct(minX, spaceW),
          topPct: pct(minY, spaceH),
          widthPct: pct(maxX - minX, spaceW),
          heightPct: pct(maxY - minY, spaceH),
        };
      });
    case 'dust':
    case 'overgrowth':
      return layer.rects.map((r) => ({
        leftPct: pct(r.x, spaceW),
        topPct: pct(r.y, spaceH),
        widthPct: pct(r.w, spaceW),
        heightPct: pct(r.h, spaceH),
      }));
  }
}
