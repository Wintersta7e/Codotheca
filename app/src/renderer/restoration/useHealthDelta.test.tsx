import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  DebtItem,
  DecayLayer,
  ProjectDetail,
  ProjectHealthDelta,
  ProjectId,
  SceneHash,
  Weathering,
} from '../../generated/protocol';
import type { RendererEvent } from '../../shared/channels';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../project/deps';
import { ProjectPageView } from '../project/ProjectPage';
import { detailFixture, NOW, rowFixture } from '../project/testFixtures';
import { shouldPlay } from './useHealthDelta';

const PROJECT = 7 as unknown as ProjectId;
const HASH = 'aa11bb22' as unknown as SceneHash;
const HERO_SRC = 'codotheca://art/aa11bb22/hero';

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

function item(layer: DecayLayer, n: number): DebtItem {
  return {
    source: 'todo_marker',
    fingerprint: `${layer}-${String(n)}`,
    state: 'open',
    scoring: 'scored',
    layer,
    pathDisplay: null,
    line: null,
    column: null,
    salientText: null,
    firstSeenAt: 1,
    lastSeenAt: 2,
    basis: 'head',
    advisory: null,
  };
}

/** Two dust anchors, so a dust value moving 2 → 1 is visible as a sprite count. */
function scene(): Weathering {
  const empty = { rects: [], points: [], paths: [] };
  return {
    projectId: PROJECT,
    sceneHash: HASH,
    spaceW: 600,
    spaceH: 900,
    layers: [
      {
        layer: 'dust',
        ...empty,
        rects: [
          { x: 48, y: 72, w: 228, h: 126 },
          { x: 324, y: 72, w: 228, h: 126 },
        ],
      },
      { layer: 'cobwebs', ...empty },
      { layer: 'rust', ...empty, points: [{ x: 150, y: 450 }] },
      { layer: 'cracks', ...empty },
      { layer: 'overgrowth', ...empty },
    ],
  };
}

function detailWith(dust: number): ProjectDetail {
  return detailFixture({
    row: rowFixture({ artSceneHash: HASH }),
    debt: Array.from({ length: dust }, (_, n) => item('dust', n)),
  });
}

function delta(detectedIn: ProjectHealthDelta['detectedIn'], id = PROJECT): ProjectHealthDelta {
  return { id, ts: NOW, detectedIn, layers: [{ layer: 'dust', fromValue: 2, toValue: 1 }] };
}

/** A page with a live `projects` subscription the test can speak into, and a swappable detail. */
function harness(initial: ProjectDetail): {
  deps: ProjectPageDeps;
  emit: (event: string, data: unknown) => void;
  setDetail: (next: ProjectDetail) => void;
} {
  let detail = initial;
  const handlers = new Set<(event: RendererEvent) => void>();
  const deps = {
    request: ((name: string) => {
      if (name === 'projects.get') return Promise.resolve(detail);
      if (name === 'art.url') return Promise.resolve(HERO_SRC);
      if (name === 'health.weathering') return Promise.resolve(scene());
      return Promise.resolve({});
    }) as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    installStart: () =>
      Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
    installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
    pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: (handler: (event: RendererEvent) => void) => {
      handlers.add(handler);
      return () => {
        handlers.delete(handler);
      };
    },
    now: () => NOW,
  } as ProjectPageDeps;
  return {
    deps,
    emit: (event, data) => {
      act(() => {
        for (const handler of [...handlers]) handler({ topic: 'projects', event, data });
      });
    },
    setDetail: (next) => {
      detail = next;
    },
  };
}

async function openPage(deps: ProjectPageDeps): Promise<void> {
  render(
    <ProjectPageDepsContext.Provider value={deps}>
      <ProjectPageView projectId={PROJECT} onBack={vi.fn()} onOpenProject={vi.fn()} />
    </ProjectPageDepsContext.Provider>,
  );
  // Settled: the bitmap is up and the anchor set has lit the layers.
  await waitFor(() => {
    expect(document.querySelector('.cdt-art')).not.toBeNull();
    expect(dustSprites()).toBeGreaterThan(0);
  });
}

function dustSprites(): number {
  return document.querySelectorAll("[data-decay-layer='dust'] .cdt-decay-sprite").length;
}

function surges(): number {
  return document.querySelectorAll('.cdt-surge').length;
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
  });
}

function setVisibility(state: DocumentVisibilityState): void {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
}

let notifications = 0;

beforeEach(() => {
  vi.stubGlobal('Image', DecodingImage);
  notifications = 0;
  // A renderer that raised one would construct this; nothing in the product may.
  vi.stubGlobal('Notification', function Notification() {
    notifications += 1;
  });
  setVisibility('visible');
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  setVisibility('visible');
});

describe('shouldPlay, the whole predicate', () => {
  it('plays a foreground delta for the mounted project on a visible document, and nothing else', () => {
    const event = (data: unknown, name = 'health_delta'): RendererEvent => ({
      topic: 'projects',
      event: name,
      data,
    });
    const cases: readonly [string, RendererEvent, DocumentVisibilityState, boolean][] = [
      ['foreground, this project, visible', event(delta('foreground')), 'visible', true],
      ['background', event(delta('background')), 'visible', false],
      ['hidden document', event(delta('foreground')), 'hidden', false],
      ['another project', event(delta('foreground', 8 as unknown as ProjectId)), 'visible', false],
      ['another event', event(delta('foreground'), 'upserted'), 'visible', false],
      ['a malformed payload', event(null), 'visible', false],
    ];
    let executed = 0;
    for (const [name, e, visibility, expected] of cases) {
      expect(shouldPlay(e, PROJECT, visibility), name).toBe(expected);
      executed += 1;
    }
    expect(executed).toBeGreaterThan(0);
  });
});

/**
 * **`AC-P3-34-9`** — a `background` delta is recorded, changes the state, and never animates:
 * not on detection, and not on the next open. A deferred replay claims a currency it does not
 * have, and the animation is a truth claim about *when* as much as about *whether*.
 */
describe('ac p3 34 9 — background changes the state and plays nothing, then or later', () => {
  it('ac_p3_34_9 plays nothing, moves the layers on upserted, notifies nobody, and never replays', async () => {
    const page = harness(detailWith(2));
    await openPage(page.deps);
    expect(dustSprites()).toBe(2);

    page.emit('health_delta', delta('background'));
    await flush();
    expect(surges(), 'a background delta animated').toBe(0);

    // The state's transport is `projects.upserted` (R121), and it moves the layers.
    page.setDetail(detailWith(1));
    page.emit('upserted', { row: { id: PROJECT } });
    await waitFor(() => {
      expect(dustSprites()).toBe(1);
    });
    expect(surges()).toBe(0);
    expect(notifications, 'the renderer raised a notification').toBe(0);

    // Open the page again and count: a background delta is never replayed on the next open.
    cleanup();
    await openPage(page.deps);
    await flush();
    console.error(`AC-P3-34-9 surges on reopen: ${String(surges())}`);
    expect(surges()).toBe(0);

    // The control that makes the zeros above mean something: the same delta, foreground, plays.
    page.emit('health_delta', delta('foreground'));
    await waitFor(() => {
      expect(surges()).toBe(1);
    });
  });
});

/**
 * **`AC-P3-34-10`** — a `foreground` delta plays nothing when the page is not mounted or the
 * document is hidden, and it is **not queued**: nothing replays when either condition clears.
 */
describe('ac p3 34 10 — no page, or a hidden document, plays nothing and queues nothing', () => {
  it('ac_p3_34_10 plays nothing while hidden and nothing when the document comes back', async () => {
    const page = harness(detailWith(2));
    await openPage(page.deps);

    setVisibility('hidden');
    page.emit('health_delta', delta('foreground'));
    await flush();
    expect(surges(), 'a hidden document animated').toBe(0);

    setVisibility('visible');
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
    await flush();
    expect(surges(), 'the hidden delta was queued and replayed').toBe(0);

    // The control: the same page, visible, does play a fresh one — the zeros above are the
    // predicate, not a surge that never works.
    page.emit('health_delta', delta('foreground'));
    await waitFor(() => {
      expect(surges()).toBe(1);
    });
  });

  it('ac_p3_34_10 plays nothing for a delta that arrived with no page, when the page mounts', async () => {
    const page = harness(detailWith(2));
    // No page is mounted, so nothing is subscribed: the event reaches no buffer at all.
    page.emit('health_delta', delta('foreground'));
    await openPage(page.deps);
    await flush();
    expect(surges(), 'a delta from before the page opened was replayed').toBe(0);

    // A different project's restoration is not this page's either.
    page.emit('health_delta', delta('foreground', 8 as unknown as ProjectId));
    await flush();
    expect(surges()).toBe(0);

    page.emit('health_delta', delta('foreground'));
    await waitFor(() => {
      expect(surges()).toBe(1);
    });
  });
});

describe('what ends a surge, and what it never carries', () => {
  it('ends at its end state the moment the user does anything', async () => {
    const page = harness(detailWith(2));
    await openPage(page.deps);
    const before = dustSprites();

    page.emit('health_delta', delta('foreground'));
    await waitFor(() => {
      expect(surges()).toBe(1);
    });
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    });
    // Immediately, not after the envelope: the end state IS the state.
    expect(surges()).toBe(0);
    // And the end state is the page with no surge at all — the layers never rode it.
    expect(dustSprites()).toBe(before);
  });

  it('leaves the rendered state to projects.upserted, never to this event', async () => {
    const page = harness(detailWith(2));
    await openPage(page.deps);

    // The delta alone moves no layer: the surge neither drives nor gates §33's rendering.
    page.setDetail(detailWith(1));
    page.emit('health_delta', delta('foreground'));
    await flush();
    expect(dustSprites()).toBe(2);

    // With every health_delta frame dropped, upserted alone moves the layers.
    page.emit('upserted', { row: { id: PROJECT } });
    await waitFor(() => {
      expect(dustSprites()).toBe(1);
    });
  });
});
