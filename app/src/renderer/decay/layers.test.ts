import { describe, expect, it } from 'vitest';
import type { DecayLayer, WeatherLayer } from '../../generated/protocol';
import { DECAY_ALPHA_MAX, DECAY_LAYER_ORDER, anchorPercents } from './layers';

const W = 600;
const H = 900;

function layer(kind: DecayLayer, over: Partial<WeatherLayer> = {}): WeatherLayer {
  return { layer: kind, rects: [], points: [], paths: [], ...over };
}

describe('the anchor conversion', () => {
  it('names the five layers in the generated enums declaration order', () => {
    expect(DECAY_LAYER_ORDER).toEqual(['dust', 'cobwebs', 'rust', 'cracks', 'overgrowth']);
  });

  it('owns the alpha ceiling as one number', () => {
    expect(DECAY_ALPHA_MAX).toBe(0.62);
  });

  it('divides by the space, with the corners at 0 and at spaceW landing on 0 and 100', () => {
    const boxes = anchorPercents(
      layer('dust', {
        rects: [
          { x: 0, y: 0, w: W, h: H },
          { x: 300, y: 450, w: 150, h: 90 },
        ],
      }),
      W,
      H,
    );
    expect(boxes[0]).toEqual({ leftPct: 0, topPct: 0, widthPct: 100, heightPct: 100 });
    expect(boxes[1]).toEqual({ leftPct: 50, topPct: 50, widthPct: 25, heightPct: 10 });
  });

  it('spans cobwebs from the anchors top-right corner to the cards', () => {
    // A module at x=48 w=228 has its top-right at 276; the card's is 600.
    const boxes = anchorPercents(
      layer('cobwebs', { rects: [{ x: 48, y: 72, w: 228, h: 126 }] }),
      W,
      H,
    );
    expect(boxes).toHaveLength(1);
    expect(boxes[0]?.leftPct).toBeCloseTo(46, 6);
    expect(boxes[0]?.topPct).toBe(0);
    expect(boxes[0]?.widthPct).toBeCloseTo(54, 6);
    expect(boxes[0]?.heightPct).toBeCloseTo(8, 6);
  });

  it('gives rust the point alone, with no size of its own', () => {
    const boxes = anchorPercents(layer('rust', { points: [{ x: 60, y: 90 }] }), W, H);
    expect(boxes[0]).toEqual({ leftPct: 10, topPct: 10, widthPct: 0, heightPct: 0 });
  });

  it('gives cracks the extent of each polyline', () => {
    const boxes = anchorPercents(
      layer('cracks', {
        paths: [
          {
            points: [
              { x: 300, y: 0 },
              { x: 300, y: 603 },
            ],
          },
        ],
      }),
      W,
      H,
    );
    expect(boxes).toHaveLength(1);
    expect(boxes[0]?.leftPct).toBe(50);
    expect(boxes[0]?.topPct).toBe(0);
    expect(boxes[0]?.widthPct).toBe(0);
    expect(boxes[0]?.heightPct).toBeCloseTo(67, 6);
  });

  it('gives overgrowth the vent rect, which decay.css hangs from its bottom edge', () => {
    const boxes = anchorPercents(
      layer('overgrowth', { rects: [{ x: 48, y: 720, w: 216, h: 36 }] }),
      W,
      H,
    );
    expect(boxes[0]?.topPct).toBeCloseTo(80, 6);
    expect(boxes[0]?.heightPct).toBe(4);
  });

  it('converts an empty anchor list to no boxes and invents no coordinate', () => {
    for (const kind of DECAY_LAYER_ORDER) {
      expect(anchorPercents(layer(kind), W, H)).toEqual([]);
    }
  });

  it('answers no box at all for a scene that has no space', () => {
    // A `Weathering` with no `art_scene` row carries `spaceW: 0` and no layers; a caller that
    // reached here anyway must not divide by it.
    expect(anchorPercents(layer('dust', { rects: [{ x: 0, y: 0, w: 1, h: 1 }] }), 0, 0)).toEqual(
      [],
    );
  });
});
