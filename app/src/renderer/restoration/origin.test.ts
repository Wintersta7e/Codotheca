import { describe, expect, it } from 'vitest';
import type { DecayLayer, ProjectId, SceneHash, Weathering } from '../../generated/protocol';
import { originFor } from './origin';

const SPACE_W = 600;
const SPACE_H = 900;

function scene(
  layers: Partial<Record<DecayLayer, Partial<Weathering['layers'][number]>>>,
  space: { w: number; h: number } = { w: SPACE_W, h: SPACE_H },
): Weathering {
  const all: DecayLayer[] = ['dust', 'cobwebs', 'rust', 'cracks', 'overgrowth'];
  return {
    projectId: 1 as unknown as ProjectId,
    sceneHash: 'scene' as unknown as SceneHash,
    spaceW: space.w,
    spaceH: space.h,
    layers: all.map((layer) => ({
      layer,
      rects: [],
      points: [],
      paths: [],
      ...layers[layer],
    })),
  };
}

/**
 * **`AC-P3-34-7`** — the origin is a **fraction of §33's scene space**, derived once, here.
 *
 * One card has three pixel spaces and the scaling between them is uniform, so a fraction is
 * exactly right in all three and a pixel figure is right in exactly one. Every space below is
 * read from the fixture, never restated in an expectation.
 */
describe('ac p3 34 7 — the origin as a fraction of the scene space', () => {
  it('ac_p3_34_7 puts an anchor at the centre of the declared space at exactly one half', () => {
    const w = scene({}).spaceW;
    const h = scene({}).spaceH;
    const centred = scene({ rust: { points: [{ x: w / 2, y: h / 2 }] } });
    expect(originFor(centred, 'rust')).toEqual({ fx: 0.5, fy: 0.5 });
  });

  it('ac_p3_34_7 takes a polyline at its first vertex and a rect at its centre', () => {
    const fixture = scene({
      cracks: {
        paths: [
          {
            points: [
              { x: 150, y: 90 },
              { x: 450, y: 810 },
            ],
          },
        ],
      },
      dust: { rects: [{ x: 60, y: 90, w: 120, h: 180 }] },
      cobwebs: { rects: [{ x: 300, y: 0, w: 300, h: 450 }] },
      overgrowth: { rects: [{ x: 0, y: 720, w: 600, h: 180 }] },
    });
    const w = fixture.spaceW;
    const h = fixture.spaceH;
    expect(originFor(fixture, 'cracks')).toEqual({ fx: 150 / w, fy: 90 / h });
    expect(originFor(fixture, 'dust')).toEqual({ fx: (60 + 120 / 2) / w, fy: (90 + 180 / 2) / h });
    expect(originFor(fixture, 'cobwebs')).toEqual({
      fx: (300 + 300 / 2) / w,
      fy: (0 + 450 / 2) / h,
    });
    expect(originFor(fixture, 'overgrowth')).toEqual({
      fx: (0 + 600 / 2) / w,
      fy: (720 + 180 / 2) / h,
    });
  });

  it('ac_p3_34_7 reads only the first entry of the list', () => {
    const fixture = scene({
      rust: {
        points: [
          { x: 60, y: 90 },
          { x: 540, y: 810 },
        ],
      },
    });
    expect(originFor(fixture, 'rust')).toEqual({
      fx: 60 / fixture.spaceW,
      fy: 90 / fixture.spaceH,
    });
  });

  /**
   * **No invented position (A1b).** An empty list, an anchor set that has not arrived and a
   * degenerate space are all *no origin*, and `(0, 0)` — a real corner of the card — is never
   * what absence looks like.
   */
  it('ac_p3_34_7 answers null, never (0, 0), when there is no origin to express', () => {
    const corner = { fx: 0, fy: 0 };
    const cases: readonly [string, Weathering | null, DecayLayer][] = [
      ['an empty anchor list', scene({}), 'dust'],
      ['a polyline with no vertex', scene({ cracks: { paths: [{ points: [] }] } }), 'cracks'],
      ['an unarrived anchor set', null, 'rust'],
      [
        'a zero-width space',
        scene({ rust: { points: [{ x: 10, y: 10 }] } }, { w: 0, h: SPACE_H }),
        'rust',
      ],
      [
        'a zero-height space',
        scene({ rust: { points: [{ x: 10, y: 10 }] } }, { w: SPACE_W, h: 0 }),
        'rust',
      ],
      ['an anchor outside the space', scene({ rust: { points: [{ x: -10, y: 10 }] } }), 'rust'],
    ];
    let executed = 0;
    for (const [name, weathering, layer] of cases) {
      const origin = originFor(weathering, layer);
      expect(origin, name).toBeNull();
      expect(origin, `${name} invented the card's corner`).not.toEqual(corner);
      executed += 1;
    }
    console.error(`AC-P3-34-7 no-origin cases executed: ${String(executed)}`);
    expect(executed).toBeGreaterThan(0);

    // A layer missing from the reply entirely is the same absence.
    const missing: Weathering = { ...scene({}), layers: [] };
    expect(originFor(missing, 'overgrowth')).toBeNull();
  });
});
