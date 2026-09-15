import type { ReactElement, ReactNode } from 'react';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../project/deps.js';

/**
 * The renderer's door, for a test that mounts a component which reaches through it.
 *
 * `ProjectCard` does: §7.6's *the request is the demand* means a rendition nothing else writes —
 * `card-blueprint` — exists only because the tile asked `art.url` for its address. The context
 * **throws** when absent rather than degrading, which is why this exists: a card mounted outside
 * the app tree must say so loudly instead of quietly painting no bitmap, since painting no
 * bitmap is exactly the shipped defect it would be hiding.
 *
 * The default `request` answers every command `undefined`, so a test that is not about art gets
 * §7.5's plate and nothing else. A test that *is* about art passes its own.
 */
export function testProjectDeps(
  request: ProjectPageDeps['request'] = (() =>
    Promise.resolve(undefined)) as ProjectPageDeps['request'],
): ProjectPageDeps {
  return {
    request,
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    subscribe: () => () => undefined,
    now: () => 1_800_000_000,
  };
}

/** A `wrapper` for `render` / `renderHook`. */
export function withProjectDeps(
  deps: ProjectPageDeps = testProjectDeps(),
): (props: { children: ReactNode }) => ReactElement {
  return ({ children }) => (
    <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
  );
}
