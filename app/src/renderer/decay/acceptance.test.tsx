import { cleanup, render, waitFor } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  DebtItem,
  DebtItemState,
  DebtScoring,
  DecayLayer,
  ProjectId,
  SceneHash,
  Weathering,
} from '../../generated/protocol';
import { HeroFrame } from '../card/HeroFrame';
import { REDUCED_CLAMP_MS } from '../motion/tier';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../project/deps';
import { rowFixture } from '../project/testFixtures';
import cardCss from '../styles/card.css?raw';
import motionCss from '../styles/motion.css?raw';
import decayCss from '../styles/decay.css?raw';
import { DecayStack } from './DecayStack';
import { litCounts } from './lit';
import { useWeathering } from './useWeathering';

afterEach(() => {
  cleanup();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
  document.body.innerHTML = '';
});

const PROJECT = 1 as unknown as ProjectId;
const SCENE = 'scene-a' as unknown as SceneHash;

function item(
  layer: DecayLayer,
  n: number,
  state: DebtItemState = 'open',
  scoring: DebtScoring = 'scored',
): DebtItem {
  return {
    source: 'todo_marker',
    fingerprint: `${layer}-${String(n)}`,
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

/** A `Screen` scene: modules face up, vents present — every layer has an anchor. */
function screenScene(): Weathering {
  return {
    projectId: PROJECT,
    sceneHash: SCENE,
    spaceW: 600,
    spaceH: 900,
    layers: [
      { layer: 'dust', rects: [rect(72), rect(216)], points: [], paths: [] },
      { layer: 'cobwebs', rects: [rect(72)], points: [], paths: [] },
      { layer: 'rust', rects: [], points: [{ x: 62, y: 86 }], paths: [] },
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
  };
}

/** A `Plain` scene with no vents — the MAJORITY shape. Only `cracks` has an anchor. */
function plainScene(): Weathering {
  return {
    projectId: PROJECT,
    sceneHash: SCENE,
    spaceW: 600,
    spaceH: 900,
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
  };
}

const ALL_OPEN: readonly DebtItem[] = [
  item('dust', 0),
  item('cobwebs', 0),
  item('rust', 0),
  item('cracks', 0),
  item('overgrowth', 0),
];

function layers(): string[] {
  return [...document.querySelectorAll('.cdt-decay')].map(
    (e) => e.getAttribute('data-decay-layer') ?? '',
  );
}

/**
 * **`AC-P3-33-1`** — the layers are lit by **items**, never by a clock.
 */
describe('ac p3 33 1 — the layer set is a function of the open debt list', () => {
  it('ac_p3_33_1 lights nothing for a project with no open items, whatever its age', () => {
    let fixtures = 0;
    // Nothing here is a date, and there is nowhere to put one: `litCounts` takes the item list
    // and no clock. A project untouched for a decade with no open item is clean metal.
    for (const debt of [[], [item('dust', 0, 'unverified')]] as readonly DebtItem[][]) {
      render(<DecayStack weathering={screenScene()} debt={debt} />);
      expect(layers()).toEqual([]);
      fixtures += 1;
      cleanup();
    }
    for (const debt of [ALL_OPEN, [item('rust', 0, 'open', 'shown_only')]]) {
      render(<DecayStack weathering={screenScene()} debt={debt} />);
      expect(layers().length).toBeGreaterThan(0);
      fixtures += 1;
      cleanup();
    }
    console.error(`AC-P3-33-1: ${String(fixtures)} fixtures scanned`);
    expect(fixtures, 'the criterion scanned no fixture').toBeGreaterThan(0);
  });
});

/**
 * **`AC-P3-33-2`** — the open-item count per layer, and **cobwebs as §28's conjunct**.
 *
 * §28's producer opens at most one `abandoned_with_debt` item and applies §28.2's conjunct,
 * narrowed to `scored` items by R122. This file **points at that predicate and restates neither
 * half** (R130/F4): every fixture is the item list §28's producer would have written.
 */
describe('ac p3 33 2 — the count is the open items, and cobwebs comes from §28', () => {
  it('ac_p3_33_2 counts open items at either scoring and never an unverified one', () => {
    const counts = litCounts([
      item('rust', 0),
      item('rust', 1, 'open', 'shown_only'),
      item('rust', 2, 'unverified'),
    ]);
    expect(counts.get('rust')).toBe(2);
  });

  it('ac_p3_33_2 never cobwebs a Done project, BECAUSE the conjunct is scored-only', () => {
    // R122: the conclusion is kept and the reason changed. A Done project CAN carry open debt —
    // one `shown_only` no-fix advisory — and it lights `rust`. It is never cobwebbed because
    // §28.2's conjunct counts `scored` items only, not because it has no open debt.
    const doneWithAdvisory = [item('rust', 0, 'open', 'shown_only')];
    render(<DecayStack weathering={screenScene()} debt={doneWithAdvisory} />);
    expect(layers()).toEqual(['rust']);
    expect(layers()).not.toContain('cobwebs');
  });
});

/**
 * **`AC-P3-33-4`** — a zero-anchor layer draws nothing and **loses no fact**.
 *
 * Asserted on `Plain` **and** on `Screen`: a gate that only built `Screen` fixtures would be
 * green while retiring the majority case from the bar.
 */
describe('ac p3 33 4 — a lit layer with no anchor draws nothing', () => {
  it('ac_p3_33_4 renders no element on Plain and the full set on Screen', () => {
    render(<DecayStack weathering={plainScene()} debt={ALL_OPEN} />);
    // Only `cracks` has an anchor on a `Plain` scene with no vents.
    expect(layers()).toEqual(['cracks']);
    cleanup();

    render(<DecayStack weathering={screenScene()} debt={ALL_OPEN} />);
    expect(layers()).toEqual(['dust', 'cobwebs', 'rust', 'cracks', 'overgrowth']);
  });

  it('ac_p3_33_4 loses no fact, because §30s list carries every item in words', () => {
    // The layer set is never the only rendering of an open item, which is what makes an
    // invisible layer a loss of ornament rather than of information.
    const counts = litCounts(ALL_OPEN);
    render(<DecayStack weathering={plainScene()} debt={ALL_OPEN} />);
    expect(layers()).toEqual(['cracks']);
    // Every one of the five is still counted; four of them simply have nowhere to hang.
    expect([...counts.keys()].sort()).toEqual(['cobwebs', 'cracks', 'dust', 'overgrowth', 'rust']);
  });
});

/**
 * **`AC-P3-33-5`** — the opened hero **and nowhere else**.
 *
 * The scope audit is a source scan and lives in `app/test/decayScope.test.ts`, in the **node**
 * project: written here as an `import.meta.glob` it raw-loads the whole renderer tree and times
 * out under the full parallel run. This is the runtime half — the stack renders nothing at all
 * unless a surface hands it one, which is what makes the default correct.
 */
describe('ac p3 33 5 — a surface that hands no stack renders none', () => {
  it('ac_p3_33_5 renders nothing when no surface supplies a weathering reply', () => {
    render(<DecayStack weathering={null} debt={ALL_OPEN} />);
    expect(layers()).toEqual([]);
  });
});

/**
 * **`AC-P3-33-6`** — the **decoded** hero is the predicate, not `art_state`.
 */
describe('ac p3 33 6 — the decoded scene hash is what mounts the layers', () => {
  function Probe(props: { hash: SceneHash | null; debt: readonly DebtItem[] }): ReactElement {
    const weathering = useWeathering(PROJECT, props.hash);
    return <DecayStack weathering={weathering} debt={props.debt} />;
  }

  function wrap(request: ProjectPageDeps['request']) {
    const deps = { request } as unknown as ProjectPageDeps;
    return function Wrapper({ children }: { children: ReactNode }): ReactElement {
      return (
        <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
      );
    };
  }

  it('ac_p3_33_6 renders zero layers for a ready hero nobody has demanded', () => {
    // `art_state` tracks the `card` rendition only, so a hero nobody has opened is `ready` with
    // NO FILE. Dusting geometry the user cannot see is a claim about a surface that is not there.
    const request = vi.fn(() => Promise.resolve(screenScene()));
    const Wrapper = wrap(request as unknown as ProjectPageDeps['request']);
    render(
      <Wrapper>
        <Probe hash={null} debt={ALL_OPEN} />
      </Wrapper>,
    );
    expect(layers()).toEqual([]);
    expect(request).not.toHaveBeenCalled();
  });

  it('ac_p3_33_6 renders them once the decoded hash matches the replys scene', async () => {
    const request = vi.fn(() => Promise.resolve(screenScene()));
    const Wrapper = wrap(request as unknown as ProjectPageDeps['request']);
    render(
      <Wrapper>
        <Probe hash={SCENE} debt={ALL_OPEN} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(layers().length).toBe(5);
    });
  });

  it('ac_p3_33_6 unmounts them the moment the scene moves, until the new one decodes', async () => {
    // A reroll (§7.4) is a new scene and a new anchor set.
    const request = vi.fn(() => Promise.resolve(screenScene()));
    const Wrapper = wrap(request as unknown as ProjectPageDeps['request']);
    const { rerender } = render(
      <Wrapper>
        <Probe hash={SCENE} debt={ALL_OPEN} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(layers().length).toBe(5);
    });
    rerender(
      <Wrapper>
        <Probe hash={'scene-rerolled' as unknown as SceneHash} debt={ALL_OPEN} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(layers()).toEqual([]);
    });
  });
});

/**
 * **`AC-P3-33-7`** — the tier clamp, **resolved on a real mounted element**.
 *
 * A clamp entry verified only by matching stylesheet text passes with the name in the wrong rule
 * group, and the wrong group is the dangerous half. The pattern is
 * `app/src/renderer/card/blueprint.test.tsx` — copied, not re-invented.
 *
 * **No assertion here matches stylesheet text and no duration is written as a literal**, because
 * the auto-format hook rewrites `.css` on write and `fmt:check` does not cover it.
 */
describe('ac p3 33 7 — the clamp resolved on a mounted .cdt-decay element', () => {
  /**
   * Mounted through the **product's own path** — `HeroFrame`'s `decay` slot — rather than as a
   * bare element. `off`'s blanket rules reach `.cdt-decay` only because it is a descendant of
   * `.cdt-card` (`motion.css`'s `[data-effects-tier='off'] .cdt-card *`), so a stack mounted
   * outside the card would sit outside that half and **nothing would say so**. The descendant
   * relation is asserted below rather than assumed.
   */
  function mountAt(tier: 'full' | 'reduced' | 'off'): Element {
    // Every stylesheet asserted non-empty FIRST: a `?raw` import that stubbed to "" would make
    // every assertion below resolve against an empty cascade and pass vacuously.
    expect(cardCss.length, 'card.css?raw imported as an empty string').toBeGreaterThan(0);
    expect(decayCss.length, 'decay.css?raw imported as an empty string').toBeGreaterThan(0);
    expect(motionCss.length, 'motion.css?raw imported as an empty string').toBeGreaterThan(0);
    const style = document.createElement('style');
    style.textContent = `${cardCss}\n${decayCss}\n${motionCss}`;
    document.head.append(style);
    document.documentElement.setAttribute('data-effects-tier', tier);
    render(
      <HeroFrame
        row={rowFixture({ artSceneHash: SCENE, artState: 'ready' })}
        heroSrc=""
        halo={{ shadow: null, opacity: 1 }}
        chips={[]}
        pin={null}
        decay={() => <DecayStack weathering={screenScene()} debt={ALL_OPEN} />}
      >
        {null}
      </HeroFrame>,
    );
    const node = document.querySelector('.cdt-decay');
    // The fixture must actually carry the element, or every assertion reads a default and
    // passes against nothing — the failure mode this whole block exists to catch.
    if (node === null) throw new Error('no .cdt-decay element mounted');
    return node;
  }

  function reset(): void {
    cleanup();
    document.head.querySelectorAll('style').forEach((n) => {
      n.remove();
    });
  }

  /** Milliseconds out of a resolved value, **as a number**. Never a string comparison. */
  function durationMs(transition: string): number {
    const match = /(\d*\.?\d+)(ms|s)\b/u.exec(transition);
    if (match === null) return 0;
    const amount = Number.parseFloat(match[1] ?? '0');
    return match[2] === 's' ? amount * 1000 : amount;
  }

  it('ac_p3_33_7 mounts inside .cdt-card, which is what puts it inside offs blanket rule', () => {
    const node = mountAt('full');
    expect(node.closest('.cdt-plate'), '.cdt-decay must sit inside the plate').not.toBeNull();
    expect(node.closest('.cdt-card'), '.cdt-decay must sit inside the card').not.toBeNull();
    reset();
  });

  it('ac_p3_33_7 renders at every tier — no tier removes a layer', () => {
    for (const tier of ['full', 'reduced', 'off'] as const) {
      const resolved = getComputedStyle(mountAt(tier));
      expect(resolved.display, tier).not.toBe('none');
      expect(resolved.opacity, tier).not.toBe('0');
      reset();
    }
  });

  it('ac_p3_33_7 drops every transform at reduced and at off, and keeps one at full', () => {
    expect(getComputedStyle(mountAt('full')).transform).not.toBe('none');
    reset();
    for (const tier of ['reduced', 'off'] as const) {
      expect(getComputedStyle(mountAt(tier)).transform, tier).toBe('none');
      reset();
    }
  });

  it('ac_p3_33_7 clamps the transition to REDUCED_CLAMP_MS, compared as a NUMBER', () => {
    // jsdom resolves the `transition` SHORTHAND and leaves `transitionDuration` at its initial
    // `0s`, so the duration is parsed out of the resolved shorthand — still a resolved style on a
    // real element, and still compared as a number read from `motion/tier.ts`.
    const full = getComputedStyle(mountAt('full')).transition;
    expect(durationMs(full)).toBeGreaterThan(REDUCED_CLAMP_MS);
    reset();

    const reduced = getComputedStyle(mountAt('reduced')).transition;
    expect(durationMs(reduced)).toBe(REDUCED_CLAMP_MS);
    expect(reduced).toContain('opacity');
    reset();

    // At `off` the blanket `.cdt-card *` rule stops it, which is why the descendant relation
    // above is asserted rather than assumed.
    expect(durationMs(getComputedStyle(mountAt('off')).transition)).toBe(0);
    reset();
  });
});
