import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  DebtItem,
  DebtItemState,
  DebtScoring,
  DecayLayer,
  ProjectId,
  SceneHash,
  Weathering,
} from '../../generated/protocol';
import { DecayStack } from './DecayStack';

afterEach(cleanup);

function item(
  layer: DecayLayer,
  index: number,
  state: DebtItemState = 'open',
  scoring: DebtScoring = 'scored',
): DebtItem {
  return {
    source: 'todo_marker',
    fingerprint: `${layer}-${String(index)}`,
    state,
    scoring,
    layer,
    pathDisplay: null,
    line: null,
    column: null,
    salientText: null,
    firstSeenAt: 1,
    lastSeenAt: 2,
    basis: 'worktree',
    advisory: null,
  };
}

function rect(y: number): { x: number; y: number; w: number; h: number } {
  return { x: 48, y, w: 228, h: 126 };
}

/** Every layer anchored, so a layer that draws nothing did so because nothing lit it. */
function weathering(over: Partial<Weathering> = {}): Weathering {
  return {
    projectId: 1 as unknown as ProjectId,
    sceneHash: 'abc' as unknown as SceneHash,
    spaceW: 600,
    spaceH: 900,
    layers: [
      { layer: 'dust', rects: [rect(72), rect(216), rect(360)], points: [], paths: [] },
      { layer: 'cobwebs', rects: [rect(72)], points: [], paths: [] },
      {
        layer: 'rust',
        rects: [],
        points: [
          { x: 62, y: 86 },
          { x: 262, y: 86 },
        ],
        paths: [],
      },
      {
        layer: 'cracks',
        rects: [],
        points: [],
        paths: [
          {
            points: [
              { x: 300, y: 0 },
              { x: 300, y: 603 },
            ],
          },
        ],
      },
      { layer: 'overgrowth', rects: [rect(720)], points: [], paths: [] },
    ],
    ...over,
  };
}

function layerEl(name: DecayLayer): HTMLElement | null {
  return document.querySelector<HTMLElement>(`.cdt-decay[data-decay-layer="${name}"]`);
}

function sprites(name: DecayLayer): number {
  return layerEl(name)?.querySelectorAll('.cdt-decay-sprite').length ?? 0;
}

describe('the decay stack', () => {
  it('renders one element per lit layer, with the five data-decay-layer values', () => {
    render(
      <DecayStack
        weathering={weathering()}
        debt={[
          item('dust', 0),
          item('cobwebs', 0),
          item('rust', 0),
          item('cracks', 0),
          item('overgrowth', 0),
        ]}
      />,
    );
    const all = [...document.querySelectorAll('.cdt-decay')].map((e) =>
      e.getAttribute('data-decay-layer'),
    );
    expect(all).toEqual(['dust', 'cobwebs', 'rust', 'cracks', 'overgrowth']);
    // One class, not five: five names in the tier clamp would be five chances to miss one.
    expect(document.querySelectorAll('.cdt-decay').length).toBe(5);
  });

  it('lights the first n anchors in order when the value is under the anchor count', () => {
    render(<DecayStack weathering={weathering()} debt={[item('dust', 0), item('dust', 1)]} />);
    expect(sprites('dust')).toBe(2);
    const first = layerEl('dust')?.querySelectorAll<HTMLElement>('.cdt-decay-sprite');
    // `rect(72)` then `rect(216)`: 72/900 = 8%, 216/900 = 24%.
    expect(first?.[0]?.style.top).toBe('8%');
    expect(first?.[1]?.style.top).toBe('24%');
  });

  it('saturates at the anchor count, so five and fifty draw the same card', () => {
    const five = Array.from({ length: 5 }, (_, i) => item('dust', i));
    const fifty = Array.from({ length: 50 }, (_, i) => item('dust', i));
    render(<DecayStack weathering={weathering()} debt={five} />);
    const atFive = layerEl('dust')?.innerHTML;
    cleanup();
    render(<DecayStack weathering={weathering()} debt={fifty} />);
    expect(layerEl('dust')?.innerHTML).toBe(atFive);
    expect(sprites('dust')).toBe(3);
  });

  it('renders NO element for a lit layer with zero anchors', () => {
    // The majority shape: a `Plain` scene with no vents resolves `dust` and `overgrowth` to zero
    // anchors. A node that claims a layer and draws nothing invites a later *a lit layer has an
    // element* assumption, which is green on `Screen` and red on `Plain`.
    const plain = weathering({
      layers: [
        { layer: 'dust', rects: [], points: [], paths: [] },
        { layer: 'cobwebs', rects: [], points: [], paths: [] },
        { layer: 'rust', rects: [], points: [], paths: [] },
        {
          layer: 'cracks',
          rects: [],
          points: [],
          paths: [
            {
              points: [
                { x: 300, y: 0 },
                { x: 300, y: 603 },
              ],
            },
          ],
        },
        { layer: 'overgrowth', rects: [], points: [], paths: [] },
      ],
    });
    render(<DecayStack weathering={plain} debt={[item('dust', 0), item('cracks', 0)]} />);
    expect(layerEl('dust')).toBeNull();
    expect(layerEl('overgrowth')).toBeNull();
    expect(sprites('cracks')).toBe(1);
  });

  it('renders no element at all for a project with no open debt', () => {
    const { container } = render(<DecayStack weathering={weathering()} debt={[]} />);
    expect(container.querySelectorAll('.cdt-decay').length).toBe(0);
  });

  it('renders nothing when there is no weathering reply to render', () => {
    const { container } = render(<DecayStack weathering={null} debt={[item('rust', 0)]} />);
    expect(container.querySelectorAll('.cdt-decay').length).toBe(0);
  });

  it('lights nothing from an unverified item', () => {
    render(<DecayStack weathering={weathering()} debt={[item('rust', 0, 'unverified')]} />);
    expect(layerEl('rust')).toBeNull();
  });

  it('lights a layer from a shown_only item', () => {
    render(<DecayStack weathering={weathering()} debt={[item('rust', 0, 'open', 'shown_only')]} />);
    expect(sprites('rust')).toBe(1);
  });

  it('carries no accessible name and no text', () => {
    render(<DecayStack weathering={weathering()} debt={[item('dust', 0)]} />);
    // The bitmap carries no text and neither does the stack; §30's page lists every item in
    // words, unconditionally, so the layers are ornament over a fact stated elsewhere.
    expect(layerEl('dust')?.getAttribute('aria-hidden')).toBe('true');
    expect(screen.queryByText(/dust/iu)).toBeNull();
  });
});
