import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { conditionDot, DOT_SIZE_PX } from '../derive/condition';
import { Identity, identityLine } from './Identity';
import { rowFixture } from './testFixtures';

afterEach(cleanup);

describe('the identity line', () => {
  it('reads condition, birth year and language', () => {
    expect(identityLine(rowFixture(), null)).toBe('IDLE · 2019 · RUST');
  });

  it('prefixes the owner only when there is one — it is NULL when it is the user', () => {
    expect(identityLine(rowFixture({ owner: 'someone-else' }), null)).toBe(
      'IDLE · SOMEONE-ELSE · 2019 · RUST',
    );
  });

  it('drops the condition word entirely when no scan job has produced a signal', () => {
    expect(identityLine(rowFixture({ conditionSignal: null }), null)).toBe('2019 · RUST');
  });

  it('omits a segment it has no value for rather than filling it', () => {
    expect(identityLine(rowFixture({ birthYear: null, primaryLanguage: null }), null)).toBe('IDLE');
    expect(
      identityLine(
        rowFixture({ conditionSignal: null, birthYear: null, primaryLanguage: null }),
        null,
      ),
    ).toBeNull();
  });
});

describe('the identity block', () => {
  it('draws exactly one dot of its own, at the project-page size', () => {
    render(<Identity visibility={null} row={rowFixture()} />);
    const dot = screen.getByTestId('cp-identity-dot');
    expect(dot.style.width).toBe(`${String(DOT_SIZE_PX.projectPage)}px`);
    expect(dot.style.height).toBe(`${String(DOT_SIZE_PX.projectPage)}px`);
  });

  it('draws no dot at all when condition_signal is NULL', () => {
    render(<Identity visibility={null} row={rowFixture({ conditionSignal: null })} />);
    expect(screen.queryByTestId('cp-identity-dot')).toBeNull();
  });

  it('does not announce the condition twice — the word is beside the mark', () => {
    render(<Identity visibility={null} row={rowFixture()} />);
    expect(screen.getByTestId('cp-identity-dot').getAttribute('aria-hidden')).toBe('true');
  });

  it('takes its colours from the one condition table, never a literal of its own', () => {
    const input = { signal: 'offline' as const, isReference: false, isArchived: false };
    const expected = conditionDot(input);
    if (expected === null) throw new Error('offline draws a dot');
    // jsdom re-serialises a hex as `rgb(...)`, so the comparison is made through the DOM on both
    // sides rather than against a retyped literal — which would also be a second copy of §5.4a.
    const probe = document.createElement('div');
    probe.style.border = expected.ring ?? '';
    render(<Identity visibility={null} row={rowFixture({ conditionSignal: 'offline' })} />);
    const dot = screen.getByTestId('cp-identity-dot');
    // §5.4a: offline is carried by the ring, with the disc only there so it does not read as
    // `empty`'s unfilled one.
    expect(dot.style.border).toBe(probe.style.border);
    expect(dot.style.border).not.toBe('');
    expect(dot.style.backgroundColor).not.toBe('');
  });

  it('renders the name and the description', () => {
    render(<Identity visibility={null} row={rowFixture()} />);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('aurora');
    expect(screen.getByTestId('cp-description').textContent).toBe(
      'A shaped description, from the manifest.',
    );
  });

  it('renders no description element when the chain produced none', () => {
    render(
      <Identity
        visibility={null}
        row={rowFixture({ description: null, descriptionSource: null })}
      />,
    );
    expect(screen.queryByTestId('cp-description')).toBeNull();
  });

  it('states no visibility — public and private are remote facts', () => {
    render(<Identity visibility={null} row={rowFixture()} />);
    expect(document.body.textContent).not.toMatch(/\b(public|private)\b/i);
  });
});

/**
 * **AC-P2-25-8.** §25.3 restores visibility to §8.5.1's identity line, after language.
 *
 * `PUBLIC` and `PRIVATE` **both render or neither does**. Rendering only `PRIVATE` makes its
 * absence assert public, which is the invented fact §8.5.1 cut the field for — and a
 * rendered-output test alone cannot see that, because a build with no `PUBLIC` branch renders
 * correctly on every `private` fixture. So the source is read as well.
 */
describe('AC-P2-25-8 visibility renders as both words or as neither', () => {
  it('appends PUBLIC after the language', () => {
    expect(identityLine(rowFixture(), 'public')).toBe('IDLE · 2019 · RUST · PUBLIC');
  });

  it('appends PRIVATE after the language', () => {
    expect(identityLine(rowFixture(), 'private')).toBe('IDLE · 2019 · RUST · PRIVATE');
  });

  it('omits the segment entirely when nothing observed it, and draws no placeholder', () => {
    const line = identityLine(rowFixture(), null);
    expect(line).toBe('IDLE · 2019 · RUST');
    expect(line).not.toContain('—');
    expect(line).not.toContain('UNKNOWN');
  });

  it('renders the word on the page, not only in the string', () => {
    render(<Identity row={rowFixture()} visibility="private" />);
    expect(screen.getByText(/PRIVATE/u)).toBeTruthy();
    cleanup();
    render(<Identity row={rowFixture()} visibility="public" />);
    expect(screen.getByText(/PUBLIC/u)).toBeTruthy();
  });

  // The source half — *a build capable of rendering `PRIVATE` while having no code path that
  // renders `PUBLIC` fails* — is asserted in `app/test/remoteScope.test.ts`, the node project:
  // this file runs in jsdom, where a source read is not expressible.

  /** §25.3: it is not on `ProjectRow`, which the 1,000-row shelf query serialises. */
  it('is absent from the shelf row', () => {
    expect(Object.keys(rowFixture())).not.toContain('visibility');
  });
});
