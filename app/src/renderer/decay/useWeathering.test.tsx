import { cleanup, render, waitFor } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  CommandArgs,
  CommandName,
  CommandResult,
  ProjectId,
  SceneHash,
  Weathering,
} from '../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../project/deps';
import { useWeathering } from './useWeathering';

afterEach(cleanup);

const PROJECT = 1 as unknown as ProjectId;

function reply(hash: string): Weathering {
  return {
    projectId: PROJECT,
    sceneHash: hash as unknown as SceneHash,
    spaceW: 600,
    spaceH: 900,
    layers: [
      { layer: 'dust', rects: [{ x: 48, y: 72, w: 228, h: 126 }], points: [], paths: [] },
      { layer: 'cobwebs', rects: [], points: [], paths: [] },
      { layer: 'rust', rects: [], points: [], paths: [] },
      { layer: 'cracks', rects: [], points: [], paths: [] },
      { layer: 'overgrowth', rects: [], points: [], paths: [] },
    ],
  };
}

interface Harness {
  readonly deps: ProjectPageDeps;
  readonly calls: () => number;
}

function harness(): Harness {
  const request = vi.fn(
    <K extends CommandName>(name: K, args: CommandArgs[K]): Promise<CommandResult[K]> => {
      if (name !== 'health.weathering') return Promise.reject(new Error(`unexpected ${name}`));
      const { projectId } = args as CommandArgs['health.weathering'];
      expect(projectId).toBe(PROJECT);
      // The core answers for the scene it has, which the caller then compares against the hash
      // on screen. The hook never tells the core which hash it wants.
      return Promise.resolve(reply('scene-a') as unknown as CommandResult[K]);
    },
  );
  const deps = { request } as unknown as ProjectPageDeps;
  return {
    deps,
    calls: () => request.mock.calls.filter(([name]) => name === 'health.weathering').length,
  };
}

function wrap(deps: ProjectPageDeps): (props: { children: ReactNode }) => ReactElement {
  return function Wrapper({ children }: { children: ReactNode }): ReactElement {
    return (
      <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
    );
  };
}

function Probe(props: {
  projectId: ProjectId | null;
  hash: SceneHash | null;
  bump?: number;
}): ReactElement {
  const weathering = useWeathering(props.projectId, props.hash);
  return (
    <div data-testid="probe" data-hash={weathering?.sceneHash ?? ''} data-bump={props.bump ?? 0} />
  );
}

function shownHash(): string {
  return document.querySelector('[data-testid="probe"]')?.getAttribute('data-hash') ?? '';
}

describe('the weathering fetch', () => {
  it('issues no request at all when nothing has decoded', () => {
    // `art_state = 'ready'` tracks the `card` rendition only, so a hero nobody has demanded is
    // `ready` with no file. Dusting geometry the user cannot see is a claim about a surface that
    // is not there — so the predicate is the DECODED hash, and a null one asks for nothing.
    const h = harness();
    const Wrapper = wrap(h.deps);
    render(
      <Wrapper>
        <Probe projectId={PROJECT} hash={null} />
      </Wrapper>,
    );
    expect(h.calls()).toBe(0);
    expect(shownHash()).toBe('');
  });

  it('issues no request without a project', () => {
    const h = harness();
    const Wrapper = wrap(h.deps);
    render(
      <Wrapper>
        <Probe projectId={null} hash={'scene-a' as unknown as SceneHash} />
      </Wrapper>,
    );
    expect(h.calls()).toBe(0);
  });

  it('answers once the decoded hash matches the replys', async () => {
    const h = harness();
    const Wrapper = wrap(h.deps);
    render(
      <Wrapper>
        <Probe projectId={PROJECT} hash={'scene-a' as unknown as SceneHash} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(shownHash()).toBe('scene-a');
    });
    expect(h.calls()).toBe(1);
  });

  it('withholds the reply while the hash on screen is a different scene', async () => {
    // A reroll (§7.4) is a new scene and a new anchor set. Until the new hero has decoded AND
    // the reply names that scene, there is nothing to position.
    const h = harness();
    const Wrapper = wrap(h.deps);
    render(
      <Wrapper>
        <Probe projectId={PROJECT} hash={'scene-b' as unknown as SceneHash} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(h.calls()).toBe(1);
    });
    expect(shownHash()).toBe('');
  });

  it('issues ONE request across three re-renders at one scene', async () => {
    // `ProjectDetail` is re-fetched on every debt change; the anchor set moves only when the art
    // re-renders. Putting the geometry on `ProjectDetail` would ship it on every item close.
    const h = harness();
    const Wrapper = wrap(h.deps);
    const hash = 'scene-a' as unknown as SceneHash;
    const { rerender } = render(
      <Wrapper>
        <Probe projectId={PROJECT} hash={hash} bump={1} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(shownHash()).toBe('scene-a');
    });
    rerender(
      <Wrapper>
        <Probe projectId={PROJECT} hash={hash} bump={2} />
      </Wrapper>,
    );
    rerender(
      <Wrapper>
        <Probe projectId={PROJECT} hash={hash} bump={3} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(shownHash()).toBe('scene-a');
    });
    expect(h.calls()).toBe(1);
  });

  it('issues a second request when the scene moves', async () => {
    const h = harness();
    const Wrapper = wrap(h.deps);
    const { rerender } = render(
      <Wrapper>
        <Probe projectId={PROJECT} hash={'scene-a' as unknown as SceneHash} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(h.calls()).toBe(1);
    });
    rerender(
      <Wrapper>
        <Probe projectId={PROJECT} hash={'scene-c' as unknown as SceneHash} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(h.calls()).toBe(2);
    });
    // The reply still names `scene-a`, so nothing is positioned over `scene-c`.
    expect(shownHash()).toBe('');
  });

  it('keeps the plate when the request fails', async () => {
    const request = vi.fn(() => Promise.reject(new Error('core restarted')));
    const deps = { request } as unknown as ProjectPageDeps;
    const Wrapper = wrap(deps);
    render(
      <Wrapper>
        <Probe projectId={PROJECT} hash={'scene-a' as unknown as SceneHash} />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(request).toHaveBeenCalledTimes(1);
    });
    expect(shownHash()).toBe('');
  });
});
