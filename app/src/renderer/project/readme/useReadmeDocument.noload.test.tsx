import { cleanup, render, screen } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { LocationId, ReadmeState } from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { NOW, rowFixture } from '../testFixtures';
import { ReadmePanel } from './ReadmePanel';

/**
 * §25.5 step 1: a README that is not `present` renders §8.5.3's existing strings and **imports
 * nothing**.
 *
 * *Nothing* is the hard half. A test that asserted "no frame rendered" would pass just as well
 * against a panel that loaded the whole markup stack and then had nothing to render with it —
 * and the whole point of the dynamic import is that those libraries stay off the first-paint
 * path. So the module is mocked with a factory that **throws**: if anything imports it, the
 * failure names why, and no ordering between tests can make this pass by accident.
 *
 * It is a file of its own because `vi.mock` is file-scoped, and every other test in this
 * directory needs the real pipeline.
 */
const pipeline = vi.hoisted(() => ({ loads: 0 }));
vi.mock('./frame', () => {
  pipeline.loads += 1;
  throw new Error('the markup pipeline must not load for a README that is not present');
});

afterEach(cleanup);

const LOCATION = 4 as unknown as LocationId;

/**
 * The bridge **answers**, and that is what makes this discriminating: with a rejecting bridge the
 * import would be skipped for the wrong reason — the request failed — and dropping the state
 * guard would still read green. Measured: it did.
 */
function withDeps(children: ReactNode, calls: string[]): ReactElement {
  const deps: ProjectPageDeps = {
    request: ((name: string) => {
      calls.push(name);
      return Promise.resolve({
        state: 'present',
        path: 'README.md',
        text: '# widget\n',
        readAt: NOW,
        truncated: false,
      });
    }) as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  return <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>;
}

describe('the pipeline is not loaded for a README there is nothing to render', () => {
  const cases: ReadonlyArray<readonly [string, ReadmeState, LocationId | null]> = [
    ['absent', { state: 'absent', text: null, readAt: NOW - 3600 }, LOCATION],
    ['not_indexed', { state: 'not_indexed', text: null, readAt: null }, LOCATION],
    // §23's zero-location project: there is no working copy to read a document from, and asking
    // would be a request about a disk that holds nothing.
    ['present with no location', { state: 'present', text: 'a paragraph', readAt: NOW }, null],
  ];

  for (const [name, readme, locationId] of cases) {
    it(`imports nothing for a ${name} README`, async () => {
      const calls: string[] = [];
      render(
        withDeps(
          <ReadmePanel readme={readme} row={rowFixture()} now={NOW} locationId={locationId} />,
          calls,
        ),
      );
      // Let every microtask the effect could have queued run before asserting an absence.
      await new Promise((resolve) => setTimeout(resolve, 0));

      expect(pipeline.loads, 'the markup chunk was loaded').toBe(0);
      expect(calls, 'a document was requested for a README there is none of').toEqual([]);
      // …and what a reader sees is §8.5.3's own string, not an empty panel.
      expect(screen.getByTestId('cp-readme-body').textContent).not.toBe('');
      expect(screen.queryByTestId('cp-readme-frame')).toBeNull();
    });
  }
});
