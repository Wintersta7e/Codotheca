import { cleanup, render, screen, waitFor } from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type { LocationId, ReadmeState } from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { DAY, NOW, rowFixture } from '../testFixtures';
import {
  README_ABSENT,
  README_NOT_INDEXED,
  README_TRUNCATED_STATEMENT,
  ReadmePanel,
  REMOTE_BLOCKED_STATEMENT,
  REMOTE_GRANT_LABEL,
} from './ReadmePanel';

afterEach(cleanup);

/**
 * [p2] §25.5: the panel now asks the core for the document, so every render needs the page's one
 * door to the outside. The default bridge **refuses**, which is the fallback §25.5 specifies —
 * the stored paragraph, which is a real fact — so every phase-1 assertion below still describes
 * what a reader sees when the document read does not answer.
 */
function withDeps(children: ReactNode, request?: ProjectPageDeps['request']): ReactElement {
  const deps: ProjectPageDeps = {
    request: request ?? (() => Promise.reject(new Error('no bridge in this test'))),
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  return <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>;
}

const LOCATION = 3 as unknown as LocationId;

const draw = (readme: ReadmeState, now = NOW): HTMLElement =>
  render(withDeps(<ReadmePanel readme={readme} row={rowFixture()} now={now} />)).container;

describe('the body', () => {
  it('renders markup as literal text — no element is created from the bytes', () => {
    const hostile = '<script>alert(1)</script> <img src=x> [link](https://example.invalid)';
    const container = draw({ state: 'present', text: hostile, readAt: NOW - 3600 });
    expect(screen.getByTestId('cp-readme-body').textContent).toBe(hostile);
    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('a')).toBeNull();
  });
});

describe('the two absences', () => {
  it('promises a pass that has not run, without v1’s dead tier vocabulary', () => {
    draw({ state: 'not_indexed', text: null, readAt: null });
    expect(screen.getByTestId('cp-readme-body').textContent).toBe(README_NOT_INDEXED);
    expect(README_NOT_INDEXED).toBe(
      'No README paragraph indexed yet — waiting on the content pass.',
    );
    expect(README_NOT_INDEXED).not.toMatch(/tier/i);
  });

  it('states a fact when the pass has run and found nothing', () => {
    draw({ state: 'absent', text: null, readAt: NOW - 3600 });
    expect(screen.getByTestId('cp-readme-body').textContent).toBe(README_ABSENT);
    expect(README_ABSENT).toBe('No README in this repository.');
  });

  it('says so rather than drawing an empty body when present carries no text', () => {
    draw({ state: 'present', text: null, readAt: NOW - 3600 });
    expect(screen.getByTestId('cp-readme-body').textContent).toBe(README_NOT_INDEXED);
  });
});

describe('the header', () => {
  it('names the file', () => {
    draw({ state: 'present', text: 'A shaped paragraph.', readAt: NOW - 3600 });
    expect(screen.getByTestId('cp-readme-name-slot').textContent).toBe('README.md');
  });

  /**
   * §6 is not suspended because the panel got bigger. `ReadmeState.readAt` is
   * `peek_cache.computed_at` on the wire (`core/src/projects/peek.rs:87-96`), so the slot has a
   * source and 14b's gap 1 is closed.
   */
  it('ages the read from the timestamp the wire carries', () => {
    draw({ state: 'present', text: 'A shaped paragraph.', readAt: NOW - 3 * DAY });
    expect(screen.getByTestId('cp-readme-age').textContent).toBe('3d');
  });

  it('draws no age at all where nothing has read, rather than a zero', () => {
    draw({ state: 'not_indexed', text: null, readAt: null });
    expect(screen.queryByTestId('cp-readme-age')).toBeNull();
    expect(screen.getByTestId('cp-readme-header').textContent).not.toMatch(/\b0\b|ago|just now/);
  });
});

describe('the two cut elements', () => {
  it('synthesises no install command and shows no topic chip', () => {
    const container = draw({ state: 'present', text: 'A shaped paragraph.', readAt: NOW - 3600 });
    expect(container.innerHTML).not.toMatch(/npm install|INSTALL|cp-readme-topics/i);
  });
});

/**
 * **AC-P2-25-9.** §25.3 restores §8.5.3's topic chips, which phase 1 cut because *"nothing local
 * supplies topics"*. One or more stored topics renders the rail; **zero renders no row at all**,
 * never an empty rail — §5.6's *nothing selected, no block renders*, and an empty rail is
 * furniture.
 */
describe('§25.3 the topic rail renders only when there is a topic', () => {
  const readme: ReadmeState = { state: 'present', text: 'A paragraph.', readAt: NOW - 3600 };

  it('renders one chip for one topic', () => {
    render(
      withDeps(<ReadmePanel readme={readme} row={rowFixture()} now={NOW} topics={['rust']} />),
    );
    const rail = screen.getByTestId('cp-readme-topics');
    expect(rail.children).toHaveLength(1);
    expect(rail.textContent).toBe('rust');
  });

  it('renders three chips for three topics', () => {
    render(
      withDeps(
        <ReadmePanel
          readme={readme}
          row={rowFixture()}
          now={NOW}
          topics={['rust', 'cli', 'tui']}
        />,
      ),
    );
    expect(screen.getByTestId('cp-readme-topics').children).toHaveLength(3);
  });

  it('AC-P2-25-9 renders no rail element at all for zero topics, asserted as an absence', () => {
    render(withDeps(<ReadmePanel readme={readme} row={rowFixture()} now={NOW} topics={[]} />));
    expect(screen.queryByTestId('cp-readme-topics')).toBeNull();
  });
});

/**
 * [p2] §25.5's panel: the document, its frame, and the three statements below it.
 *
 * The **ordering** assertion is the one this file exists for: `srcdoc` is set once with no `src`
 * attribute anywhere before `projects.readmeAssets` resolves, and again after. That is
 * AC-P2-25-18's request census one level down and far cheaper to run — an image that reached the
 * frame before the core vetted it would be a request from inside the sandbox.
 */
describe('§25.5 the panel renders the document in a frame', () => {
  const present: ReadmeState = { state: 'present', text: 'stored paragraph', readAt: NOW - 3600 };

  /**
   * The first frame waits on a real dynamic `import()` of the whole markup stack — parser,
   * sanitiser, highlighter, typesetter — which is slower than the library's one-second default
   * when the suite is running the rest of the renderer beside it. Measured: 1005 ms, failing only
   * in the full run. The wait is generous rather than tuned; what is under test is the ordering,
   * not the load time.
   */
  const SETTLE = { timeout: 20_000 };

  function bridge(
    document: string,
    assets: unknown[] = [],
  ): {
    request: ProjectPageDeps['request'];
    calls: string[];
    release: () => void;
  } {
    const calls: string[] = [];
    let releaseAssets = (): void => undefined;
    const held = new Promise<void>((resolve) => {
      releaseAssets = () => {
        resolve();
      };
    });
    const request = (async (name: string) => {
      calls.push(name);
      if (name === 'projects.readme') {
        return {
          state: 'present',
          path: 'README.rst',
          text: document,
          readAt: NOW - 3600,
          truncated: true,
        };
      }
      if (name === 'projects.readmeAssets') {
        await held;
        return assets;
      }
      return {};
    }) as unknown as ProjectPageDeps['request'];
    return { request, calls, release: releaseAssets };
  }

  it('sets the first srcdoc with no src attribute anywhere, then substitutes', async () => {
    const { request, calls, release } = bridge(
      '# widget\n\n![badge](https://cdn.example.test/badge.svg)\n\n[docs](https://example.test)\n',
      [
        {
          ref: 'https://cdn.example.test/badge.svg',
          state: 'ok',
          dataUri: 'data:image/png;base64,iVBORw0KGgo=',
          fetchedAt: NOW,
        },
      ],
    );
    render(
      withDeps(
        <ReadmePanel readme={present} row={rowFixture()} now={NOW} locationId={LOCATION} />,
        request,
      ),
    );

    const frame = await screen.findByTestId('cp-readme-frame', undefined, SETTLE);
    const first = frame.getAttribute('srcdoc') ?? '';
    expect(first).toContain('widget');
    expect(first).not.toContain('src=');
    expect(first).toContain('cdt-readme-asset');
    expect(calls).toContain('projects.readme');

    // Document 1 is the first paint, and it says so. The number is what the end-to-end census
    // names to read a settled document rather than racing the element's replacement, so it has an
    // owner here rather than only in a spec that takes four minutes to tell anyone it is gone.
    expect(frame.getAttribute('data-revision')).toBe('1');

    release();
    await waitFor(() => {
      expect(screen.getByTestId('cp-readme-frame').getAttribute('srcdoc') ?? '').toContain(
        'data:image/png;base64,',
      );
    }, SETTLE);
    expect(calls).toEqual(['projects.readme', 'projects.readmeAssets']);
    // …and the substituted document is the second, which is what makes it a different element:
    // Chromium does not re-navigate a sandboxed frame when `srcdoc` is replaced.
    expect(screen.getByTestId('cp-readme-frame').getAttribute('data-revision')).toBe('2');
  });

  /**
   * **AC-P2-25-16, on the element the panel renders.**
   *
   * The whole threat model rests on this one attribute: present and **exactly empty** is an opaque
   * origin with every flag off, and script execution dead twice over. The assertion has to read it
   * off what `ReadmePanel` produced — an earlier version built its own `iframe` and asserted that
   * `setAttribute` works, under which deleting the attribute from the component left every gate in
   * the repository green.
   */
  it('readmeFrame::ac_p2_25_16_the_sandbox_attribute_is_present_and_empty', async () => {
    const { request, release } = bridge('# widget\n');
    render(
      withDeps(
        <ReadmePanel readme={present} row={rowFixture()} now={NOW} locationId={LOCATION} />,
        request,
      ),
    );
    release();
    const frame = await screen.findByTestId('cp-readme-frame', undefined, SETTLE);

    expect(frame.tagName.toLowerCase()).toBe('iframe');
    expect(frame.hasAttribute('sandbox')).toBe(true);
    expect(frame.getAttribute('sandbox')).toBe('');
    // Every flag, by name, off the rendered attribute — so widening it fails as loudly as
    // removing it.
    for (const flag of [
      'allow-scripts',
      'allow-same-origin',
      'allow-forms',
      'allow-popups',
      'allow-top-navigation',
      'allow-downloads',
      'allow-modals',
      'allow-pointer-lock',
      'allow-presentation',
      'allow-orientation-lock',
    ]) {
      expect((frame.getAttribute('sandbox') ?? '').includes(flag), flag).toBe(false);
    }
    // …and the frame is carrying a document, so this is not an assertion about an empty element.
    expect((frame.getAttribute('srcdoc') ?? '').length).toBeGreaterThan(100);
  });

  it('states that the document was cut, and that its links are inert', async () => {
    const { request, release } = bridge('# widget\n\n[docs](https://example.test)\n');
    render(
      withDeps(
        <ReadmePanel readme={present} row={rowFixture()} now={NOW} locationId={LOCATION} />,
        request,
      ),
    );
    release();
    await screen.findByTestId('cp-readme-frame', undefined, SETTLE);
    expect(screen.getByTestId('cp-readme-truncated').textContent).toBe(README_TRUNCATED_STATEMENT);
    // Exactly once, however many anchors the document has.
    expect(screen.getAllByTestId('cp-readme-links-inert')).toHaveLength(1);
    // …and the stored paragraph is gone: the frame replaced it rather than joining it.
    expect(screen.queryByTestId('cp-readme-body')).toBeNull();
  });

  it('draws no inert-links statement for a document with no anchors', async () => {
    const { request, release } = bridge('# widget\n\nJust prose.\n');
    render(
      withDeps(
        <ReadmePanel readme={present} row={rowFixture()} now={NOW} locationId={LOCATION} />,
        request,
      ),
    );
    release();
    await screen.findByTestId('cp-readme-frame', undefined, SETTLE);
    expect(screen.queryByTestId('cp-readme-links-inert')).toBeNull();
  });

  it('offers the consent as a sentence and a control, with no failure ink', async () => {
    const { request, release } = bridge('![badge](https://cdn.example.test/badge.svg)\n', [
      {
        ref: 'https://cdn.example.test/badge.svg',
        state: 'blocked',
        dataUri: null,
        fetchedAt: null,
      },
    ]);
    render(
      withDeps(
        <ReadmePanel readme={present} row={rowFixture()} now={NOW} locationId={LOCATION} />,
        request,
      ),
    );
    release();
    const consent = await screen.findByTestId('cp-readme-consent', undefined, SETTLE);
    expect(consent.textContent).toContain(REMOTE_BLOCKED_STATEMENT);
    expect(consent.textContent).toContain(REMOTE_GRANT_LABEL);
    // `blocked` is the consent state: no failure vocabulary anywhere in the panel.
    expect(consent.textContent).not.toMatch(/fail|error|unreachable|refused/iu);
  });

  it('names the file the core actually read', async () => {
    const { request, release } = bridge('# widget\n');
    render(
      withDeps(
        <ReadmePanel readme={present} row={rowFixture()} now={NOW} locationId={LOCATION} />,
        request,
      ),
    );
    release();
    await screen.findByTestId('cp-readme-frame', undefined, SETTLE);
    expect(screen.getByTestId('cp-readme-name-slot').textContent).toBe('README.rst');
  });

  it('keeps the stored paragraph when the document read refuses', async () => {
    const request = (() =>
      Promise.reject(new Error('REPO_UNREADABLE'))) as unknown as ProjectPageDeps['request'];
    render(
      withDeps(
        <ReadmePanel readme={present} row={rowFixture()} now={NOW} locationId={LOCATION} />,
        request,
      ),
    );
    await waitFor(() => {
      expect(screen.getByTestId('cp-readme-body').textContent).toBe('stored paragraph');
    }, SETTLE);
    expect(screen.queryByTestId('cp-readme-frame')).toBeNull();
  });
});
