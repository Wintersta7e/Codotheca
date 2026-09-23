import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  DebtItem,
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
import cardCss from '../styles/card.css?raw';
import { RESTORATION_SURGE_MS } from './envelope';

const PROJECT = 7 as unknown as ProjectId;
const HASH = 'aa11bb22' as unknown as SceneHash;

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

function dust(n: number): DebtItem {
  return {
    source: 'missing_readme',
    fingerprint: `dust-${String(n)}`,
    state: 'open',
    scoring: 'scored',
    layer: 'dust',
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

function scene(): Weathering {
  const empty = { rects: [], points: [], paths: [] };
  return {
    projectId: PROJECT,
    sceneHash: HASH,
    spaceW: 600,
    spaceH: 900,
    layers: [
      { layer: 'dust', ...empty, rects: [{ x: 48, y: 72, w: 228, h: 126 }] },
      { layer: 'cobwebs', ...empty },
      { layer: 'rust', ...empty },
      { layer: 'cracks', ...empty },
      { layer: 'overgrowth', ...empty },
    ],
  };
}

const DETAIL: ProjectDetail = detailFixture({
  row: rowFixture({ artSceneHash: HASH }),
  debt: [dust(0), dust(1)],
});

const FOREGROUND: ProjectHealthDelta = {
  id: PROJECT,
  ts: NOW,
  detectedIn: 'foreground',
  layers: [{ layer: 'dust', fromValue: 2, toValue: 1 }],
};

function openPage(): (event: RendererEvent) => void {
  const handlers = new Set<(event: RendererEvent) => void>();
  const deps = {
    request: ((name: string) => {
      if (name === 'projects.get') return Promise.resolve(DETAIL);
      if (name === 'art.url') return Promise.resolve('codotheca://art/aa11bb22/hero');
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
  render(
    <ProjectPageDepsContext.Provider value={deps}>
      <ProjectPageView projectId={PROJECT} onBack={vi.fn()} onOpenProject={vi.fn()} />
    </ProjectPageDepsContext.Provider>,
  );
  return (event) => {
    act(() => {
      for (const handler of [...handlers]) handler(event);
    });
  };
}

let frames = 0;

/**
 * Everything the page has scheduled that is animation: frames it asked for, timers the surge is
 * holding, and surge elements mounted with a resolved animation. **Counted, never read out of
 * CSS**: jsdom runs no CSS animation, so what can be counted is what the page schedules.
 */
function scheduled(baselineTimers: number): {
  frames: number;
  timers: number;
  animating: number;
} {
  const animating = [...document.querySelectorAll('.cdt-surge')].filter(
    (node) => getComputedStyle(node).animationName !== 'none',
  ).length;
  return { frames, timers: vi.getTimerCount() - baselineTimers, animating };
}

beforeEach(() => {
  vi.stubGlobal('Image', DecodingImage);
  frames = 0;
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    frames += 1;
    return window.setTimeout(() => {
      callback(0);
    }, 0);
  });
  expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
  const style = document.createElement('style');
  style.textContent = cardCss;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', 'full');
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
  vi.unstubAllGlobals();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
});

/** Open the page with real timers until it has settled, then hand the clock to the test. */
async function settledPage(): Promise<(event: RendererEvent) => void> {
  const emit = openPage();
  await waitFor(() => {
    expect(document.querySelector('.cdt-art')).not.toBeNull();
    expect(document.querySelector("[data-decay-layer='dust']")).not.toBeNull();
  });
  await act(async () => {
    await Promise.resolve();
  });
  vi.useFakeTimers();
  frames = 0;
  return emit;
}

/**
 * **`AC-P3-34-17`** — the zero-animation-frames sentence is a ban on a *schedule*, not on a
 * *response*. An opened page schedules nothing once its entry has played; a foreground delta —
 * an event the user is present for — plays one bounded, one-shot surge that ends, and the page is
 * back at nothing. The unbounded, uncaused drift that sentence killed stays dead.
 */
describe('ac p3 34 17 — zero, then one bounded surge, then zero', () => {
  it('ac_p3_34_17 counts nothing at idle, one bounded one-shot on a delta, nothing after', async () => {
    const emit = await settledPage();
    const baseline = vi.getTimerCount();

    const idle = scheduled(baseline);
    console.error(`AC-P3-34-17 idle: ${JSON.stringify(idle)}`);
    expect(idle).toEqual({ frames: 0, timers: 0, animating: 0 });

    emit({ topic: 'projects', event: 'health_delta', data: FOREGROUND });
    const during = scheduled(baseline);
    console.error(`AC-P3-34-17 during: ${JSON.stringify(during)}`);
    // One element animating, one timer bounding it, and no frame loop driving it.
    expect(during).toEqual({ frames: 0, timers: 1, animating: 1 });

    // Short of the envelope it is still playing; at the envelope it has ended by itself.
    act(() => {
      vi.advanceTimersByTime(RESTORATION_SURGE_MS - 1);
    });
    expect(scheduled(baseline).animating).toBe(1);
    act(() => {
      vi.advanceTimersByTime(1);
    });
    const after = scheduled(baseline);
    console.error(`AC-P3-34-17 after: ${JSON.stringify(after)}`);
    expect(after).toEqual({ frames: 0, timers: 0, animating: 0 });
  });

  it('ac_p3_34_17 returns to zero at once on user input, without waiting out the envelope', async () => {
    const emit = await settledPage();
    const baseline = vi.getTimerCount();

    emit({ topic: 'projects', event: 'health_delta', data: FOREGROUND });
    expect(scheduled(baseline).animating).toBe(1);

    act(() => {
      window.dispatchEvent(new Event('pointerdown'));
    });
    expect(scheduled(baseline)).toEqual({ frames: 0, timers: 0, animating: 0 });
  });
});
