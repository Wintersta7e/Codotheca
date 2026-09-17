import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { createElement, type ReactElement, type ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';

import type { LocationId, ReadmeState } from '../../src/generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../../src/renderer/project/deps';
import { ReadmePanel } from '../../src/renderer/project/readme/ReadmePanel';
import { NOW, rowFixture } from '../../src/renderer/project/testFixtures';

/**
 * [p2] §25.10's panel-level bar: what a reader sees, rather than what a module returns.
 *
 * The per-module suites beside the code assert the pipeline, the frame and the hostile document.
 * What is left for this file is the panel as a whole — both paragraphs of a real document inside
 * the frame, no `UNKNOWN` anywhere, and no element carrying a link affordance.
 *
 * It lives in `test/dom/` because `app/test/**` is the **node** project by default and this needs
 * a DOM; `vitest.config.ts` gives `test/dom/**` jsdom for exactly this.
 */
afterEach(cleanup);

const LOCATION = 9 as unknown as LocationId;
const SETTLE = { timeout: 20_000 };

const DOCUMENT = [
  '# widget',
  '',
  'The first paragraph, which the stored excerpt also carries.',
  '',
  'The second paragraph, which only the document has.',
  '',
  '[docs](https://example.test/docs)',
  '',
].join('\n');

function withDeps(children: ReactNode, request: ProjectPageDeps['request']): ReactElement {
  const deps: ProjectPageDeps = {
    request,
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  return createElement(ProjectPageDepsContext.Provider, { value: deps }, children);
}

const answering = ((name: string) => {
  if (name === 'projects.readme') {
    return Promise.resolve({
      state: 'present',
      path: 'README.md',
      text: DOCUMENT,
      readAt: NOW - 3600,
      truncated: false,
    });
  }
  return Promise.resolve([]);
}) as unknown as ProjectPageDeps['request'];

const stored: ReadmeState = {
  state: 'present',
  text: 'The first paragraph, which the stored excerpt also carries.',
  readAt: NOW - 3600,
};

describe('§25.5 the README panel, as a reader sees it', () => {
  it('renders both paragraphs of the document inside the frame', async () => {
    render(
      withDeps(
        createElement(ReadmePanel, {
          readme: stored,
          row: rowFixture(),
          now: NOW,
          locationId: LOCATION,
        }),
        answering,
      ),
    );
    const frame = await screen.findByTestId('cp-readme-frame', undefined, SETTLE);
    const srcdoc = frame.getAttribute('srcdoc') ?? '';
    expect(srcdoc).toContain('The first paragraph, which the stored excerpt also carries.');
    expect(srcdoc).toContain('The second paragraph, which only the document has.');
    // The stored paragraph carries only the first, which is why this command exists at all.
    expect(stored.text).not.toContain('only the document has');
  });

  it('renders the string UNKNOWN zero times', async () => {
    const { container } = render(
      withDeps(
        createElement(ReadmePanel, {
          readme: stored,
          row: rowFixture(),
          now: NOW,
          locationId: LOCATION,
        }),
        answering,
      ),
    );
    await screen.findByTestId('cp-readme-frame', undefined, SETTLE);
    expect(container.innerHTML).not.toContain('UNKNOWN');
    expect(container.textContent ?? '').not.toContain('UNKNOWN');
  });

  it('carries no link affordance in the panel itself', async () => {
    const { container } = render(
      withDeps(
        createElement(ReadmePanel, {
          readme: stored,
          row: rowFixture(),
          now: NOW,
          locationId: LOCATION,
        }),
        answering,
      ),
    );
    await screen.findByTestId('cp-readme-frame', undefined, SETTLE);
    // No anchor, and nothing claiming one: the document's own anchors live inside a frame that
    // cannot navigate, and the panel states that below it instead of drawing a dead control.
    expect(container.querySelectorAll('a')).toHaveLength(0);
    await waitFor(() => {
      expect(screen.getByTestId('cp-readme-links-inert').textContent).toBe(
        'LINKS ARE NOT ACTIVE IN THIS PANEL',
      );
    }, SETTLE);
  });
});
