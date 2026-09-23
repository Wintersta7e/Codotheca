import { act, cleanup, render, waitFor } from '@testing-library/react';
import { useState, type ReactElement } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DecayLayer, ProjectId, SceneHash, Weathering } from '../../generated/protocol';
import { CardPlate } from '../card/CardPlate';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../project/deps';
import { HeroTile } from '../project/hero/HeroTile';
import { NOW, rowFixture } from '../project/testFixtures';
import cardCss from '../styles/card.css?raw';
import type { Selection } from './select';
import surgeSource from './Surge.tsx?raw';
import type { SurgeRequest } from './Surge';

const HASH = 'aa11bb22' as unknown as SceneHash;
const HERO_SRC = 'codotheca://art/aa11bb22/hero';

/** A detached `Image` that decodes whatever it is handed, so the hero's `.cdt-art` mounts. */
class DecodingImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  #src = '';
  get src(): string {
    return this.#src;
  }
  set src(value: string) {
    this.#src = value;
    queueMicrotask(() => this.onload?.());
  }
}

function layer(
  name: DecayLayer,
  over: Partial<Weathering['layers'][number]> = {},
): Weathering['layers'][number] {
  return { layer: name, rects: [], points: [], paths: [], ...over };
}

/** A `Screen`-shaped reply: every layer has an anchor. */
function screenScene(): Weathering {
  return {
    projectId: 7 as unknown as ProjectId,
    sceneHash: HASH,
    spaceW: 600,
    spaceH: 900,
    layers: [
      layer('dust', { rects: [{ x: 48, y: 72, w: 228, h: 126 }] }),
      layer('cobwebs', { rects: [{ x: 48, y: 72, w: 228, h: 126 }] }),
      layer('rust', { points: [{ x: 150, y: 450 }] }),
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
      layer('overgrowth', { rects: [{ x: 0, y: 720, w: 600, h: 180 }] }),
    ],
  };
}

/**
 * The **majority** shape: a scene whose modules are all `ModuleKind::Plain` and which has no
 * vents. `face_up()` matches three of eight kinds, so this scene offers `dust`, `cobwebs`, `rust`
 * and `overgrowth` no anchor at all; only the always-drawn seam gives `cracks` one.
 */
function plainScene(): Weathering {
  return {
    ...screenScene(),
    layers: [
      layer('dust'),
      layer('cobwebs'),
      layer('rust'),
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
      layer('overgrowth'),
    ],
  };
}

function deps(weathering: () => Promise<Weathering>): {
  deps: ProjectPageDeps;
  request: ReturnType<typeof vi.fn>;
} {
  const request = vi.fn((name: string) => {
    if (name === 'art.url') return Promise.resolve(HERO_SRC);
    if (name === 'health.weathering') return weathering();
    return Promise.reject(new Error(`unexpected ${name}`));
  });
  return {
    request,
    deps: {
      request: request as unknown as ProjectPageDeps['request'],
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
      installStart: () =>
        Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
      installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
      pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
      openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
      subscribe: () => () => undefined,
      now: () => NOW,
    },
  };
}

/** The live request's two halves, as the page holds them. Set by `Harness` on every render. */
let startSurge: ((selection: Selection) => void) | null = null;
let endSurge: (() => void) | null = null;

/**
 * The hero as the page mounts it: **idle first**, the surge slot holding a request whose
 * selection is `none` — which is how `ProjectPage` holds it between events, so the anchor set has
 * arrived by the time a restoration does.
 */
function Harness(): ReactElement {
  const [selection, setSelection] = useState<Selection>({ kind: 'none' });
  const request: SurgeRequest = {
    selection,
    end: () => {
      setSelection({ kind: 'none' });
    },
  };
  startSurge = setSelection;
  endSurge = request.end;
  return (
    <HeroTile
      row={rowFixture({ artSceneHash: HASH })}
      heroHash={HASH}
      firstRunCompletedAt={null}
      isPinned={false}
      onTogglePin={() => undefined}
      surge={request}
    />
  );
}

/** Mount the idle hero, let its bitmap decode and its anchor request go out, then play. */
async function play(selection: Selection, weathering: () => Promise<Weathering>): Promise<void> {
  const { deps: pageDeps, request } = deps(weathering);
  render(
    <ProjectPageDepsContext.Provider value={pageDeps}>
      <Harness />
    </ProjectPageDepsContext.Provider>,
  );
  await art();
  await waitFor(() => {
    expect(request).toHaveBeenCalledWith('health.weathering', expect.anything());
  });
  await act(async () => {
    await Promise.resolve();
  });
  act(() => {
    startSurge?.(selection);
  });
}

function styleAtFull(): void {
  expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
  const style = document.createElement('style');
  style.textContent = cardCss;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', 'full');
}

async function art(): Promise<Element> {
  let found: Element | null = null;
  await waitFor(() => {
    found = document.querySelector('.cdt-art');
    expect(found, 'the hero never decoded its bitmap').not.toBeNull();
  });
  return found as unknown as Element;
}

beforeEach(() => {
  vi.stubGlobal('Image', DecodingImage);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
  document.body.innerHTML = '';
});

/**
 * **`AC-P3-34-1`** — the surge is DOM, placed where §34.1 puts it, and it paints no canvas.
 *
 * Placement is asserted by walking the tree, not by matching a class list: a surge mounted
 * outside `.cdt-plate` would sit outside the card subtree `motion.css`'s `off` blanket reaches.
 */
describe('ac p3 34 1 — one DOM element inside the hero plate', () => {
  it('ac_p3_34_1 is a sibling of .cdt-art inside .cdt-plate, and no canvas is ever made', async () => {
    const getContext = vi.spyOn(HTMLCanvasElement.prototype, 'getContext');
    await play({ kind: 'layer', layer: 'rust' }, () => Promise.resolve(screenScene()));
    const bitmap = await art();
    const surge = await waitFor(() => {
      const node = document.querySelector('.cdt-surge');
      expect(node).not.toBeNull();
      return node as Element;
    });

    const plate = bitmap.parentElement;
    expect(plate?.classList.contains('cdt-plate'), '.cdt-art is not a child of .cdt-plate').toBe(
      true,
    );
    expect(surge.parentElement, 'the surge is not a sibling of .cdt-art').toBe(plate);
    expect(surge.closest('.cdt-card'), 'the surge sits outside the card subtree').not.toBeNull();

    // Run it to completion, then count — the negative rule is about what a surge leaves behind.
    // jsdom runs no CSS animation, so completion is the callback the animation's own end (or
    // the envelope's timer, `useHealthDelta`'s) makes.
    act(() => {
      endSurge?.();
    });
    await waitFor(() => {
      expect(document.querySelector('.cdt-surge')).toBeNull();
    });
    const canvases = document.querySelectorAll('canvas').length;
    console.error(
      `AC-P3-34-1 after completion: ${String(canvases)} canvas, ${String(getContext.mock.calls.length)} getContext call(s)`,
    );
    expect(canvases).toBe(0);
    expect(getContext).not.toHaveBeenCalled();
  });

  it('ac_p3_34_1 is mounted by nothing when the slot is empty — the grid passes none', () => {
    render(
      <CardPlate surface="card" isArchived={false} art={null}>
        {null}
      </CardPlate>,
    );
    expect(document.querySelector('.cdt-plate')).not.toBeNull();
    expect(document.querySelector('.cdt-surge')).toBeNull();
  });
});

/**
 * **`AC-P3-34-6`** — §34.5's four outcomes, rendered four ways, on a mounted page.
 *
 * Each rendering differs in `data-surge` **and** in a resolved `animation-name`, so the test cannot
 * pass on an attribute the stylesheet ignores.
 */
describe('ac p3 34 6 — four outcomes, no two collapsed', () => {
  async function outcome(
    selection: Selection,
    weathering: () => Promise<Weathering>,
  ): Promise<{ surge: string; animation: string; x: string; y: string } | null> {
    styleAtFull();
    await play(selection, weathering);
    const node = document.querySelector<HTMLElement>('.cdt-surge');
    const result =
      node === null
        ? null
        : {
            surge: node.getAttribute('data-surge') ?? '',
            animation: getComputedStyle(node).animationName,
            x: node.style.getPropertyValue('--cdt-surge-x'),
            y: node.style.getPropertyValue('--cdt-surge-y'),
          };
    cleanup();
    document.head.querySelectorAll('style').forEach((n) => {
      n.remove();
    });
    return result;
  }

  it('ac_p3_34_6 renders nothing, the whole card, the wipe and the point — four ways', async () => {
    const scene = (): Promise<Weathering> => Promise.resolve(screenScene());
    const none = await outcome({ kind: 'none' }, scene);
    const whole = await outcome({ kind: 'whole' }, scene);
    const wipe = await outcome({ kind: 'layer', layer: 'dust' }, () =>
      Promise.resolve(plainScene()),
    );
    const point = await outcome({ kind: 'layer', layer: 'rust' }, scene);

    expect(none, 'no decrease mounted an element').toBeNull();
    const rendered = [whole, wipe, point];
    for (const r of rendered) {
      expect(r, 'a surge outcome mounted nothing').not.toBeNull();
      expect(r?.animation, 'an outcome resolved no animation').not.toBe('');
      expect(r?.animation).not.toBe('none');
    }
    expect(rendered.map((r) => r?.surge)).toEqual(['whole', 'wipe', 'point']);
    const names = new Set(rendered.map((r) => r?.animation));
    console.error(`AC-P3-34-6 resolved animations: ${[...names].join(' ')}`);
    expect(names.size, 'two outcomes resolve the same animation').toBe(rendered.length);

    // The point is the fixture's rust fastener as a fraction of the space, as a percentage.
    expect(point?.x).toBe(`${String((150 / 600) * 100)}%`);
    expect(point?.y).toBe(`${String((450 / 900) * 100)}%`);
  });

  it('ac_p3_34_6 renders the wipe on a Plain scene and on an unarrived anchor set, never (0, 0)', async () => {
    const plain = await outcome({ kind: 'layer', layer: 'dust' }, () =>
      Promise.resolve(plainScene()),
    );
    // A reply that never lands: `health.weathering` unresolved is the same absence.
    const unarrived = await outcome(
      { kind: 'layer', layer: 'rust' },
      () => new Promise<Weathering>(() => undefined),
    );
    for (const [name, r] of [
      ['plain', plain],
      ['unarrived', unarrived],
    ] as const) {
      expect(r?.surge, `${name} did not render the wipe`).toBe('wipe');
      // No custom property at all: the wipe carries no position, least of all the corner.
      expect(r?.x, `${name} carried an x`).toBe('');
      expect(r?.y, `${name} carried a y`).toBe('');
      expect(r?.x).not.toBe('0%');
    }
  });
});

/**
 * **`AC-P3-34-7`'s placement clause**: *the module positioning the surge declares no numeric
 * literal other than `0`, `1` and `100`*. This audits `Surge.tsx`, the module that positions it;
 * `origin.ts` is the derivation and carries the halving §34.5's rect-centre formula states.
 */
describe('ac p3 34 7 — the positioning module multiplies by no pixel constant', () => {
  it('ac_p3_34_7 finds no numeric literal in Surge.tsx but 0, 1 and 100', () => {
    expect(surgeSource.length, 'Surge.tsx read as an empty string').toBeGreaterThan(0);
    const code = surgeSource
      .replace(/\/\*[\s\S]*?\*\//gu, '')
      .replace(/\/\/[^\n]*/gu, '')
      .replace(/'[^'\n]*'|"[^"\n]*"/gu, "''");
    const literals = [...code.matchAll(/(?<![\w.$-])\d+(?:\.\d+)?(?![\w.])/gu)].map((m) => m[0]);
    console.error(`AC-P3-34-7 Surge.tsx numeric literals: ${literals.join(' ') || '(none)'}`);
    expect(
      literals.length,
      'the audit found no literal at all — it read the wrong file',
    ).toBeGreaterThan(0);
    for (const literal of literals) {
      expect(['0', '1', '100'], `Surge.tsx declares ${literal}`).toContain(literal);
    }
  });
});
