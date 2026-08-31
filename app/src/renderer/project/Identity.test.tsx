import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { conditionDot, DOT_SIZE_PX } from '../derive/condition';
import { Identity, identityLine } from './Identity';
import { rowFixture } from './testFixtures';

afterEach(cleanup);

describe('the identity line', () => {
  it('reads condition, birth year and language', () => {
    expect(identityLine(rowFixture())).toBe('IDLE · 2019 · RUST');
  });

  it('prefixes the owner only when there is one — it is NULL when it is the user', () => {
    expect(identityLine(rowFixture({ owner: 'someone-else' }))).toBe(
      'IDLE · SOMEONE-ELSE · 2019 · RUST',
    );
  });

  it('drops the condition word entirely when no scan job has produced a signal', () => {
    expect(identityLine(rowFixture({ conditionSignal: null }))).toBe('2019 · RUST');
  });

  it('omits a segment it has no value for rather than filling it', () => {
    expect(identityLine(rowFixture({ birthYear: null, primaryLanguage: null }))).toBe('IDLE');
    expect(
      identityLine(rowFixture({ conditionSignal: null, birthYear: null, primaryLanguage: null })),
    ).toBeNull();
  });
});

describe('the identity block', () => {
  it('draws exactly one dot of its own, at the project-page size', () => {
    render(<Identity row={rowFixture()} />);
    const dot = screen.getByTestId('cp-identity-dot');
    expect(dot.style.width).toBe(`${String(DOT_SIZE_PX.projectPage)}px`);
    expect(dot.style.height).toBe(`${String(DOT_SIZE_PX.projectPage)}px`);
  });

  it('draws no dot at all when condition_signal is NULL', () => {
    render(<Identity row={rowFixture({ conditionSignal: null })} />);
    expect(screen.queryByTestId('cp-identity-dot')).toBeNull();
  });

  it('does not announce the condition twice — the word is beside the mark', () => {
    render(<Identity row={rowFixture()} />);
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
    render(<Identity row={rowFixture({ conditionSignal: 'offline' })} />);
    const dot = screen.getByTestId('cp-identity-dot');
    // §5.4a: offline is carried by the ring, with the disc only there so it does not read as
    // `empty`'s unfilled one.
    expect(dot.style.border).toBe(probe.style.border);
    expect(dot.style.border).not.toBe('');
    expect(dot.style.backgroundColor).not.toBe('');
  });

  it('renders the name and the description', () => {
    render(<Identity row={rowFixture()} />);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('aurora');
    expect(screen.getByTestId('cp-description').textContent).toBe(
      'A shaped description, from the manifest.',
    );
  });

  it('renders no description element when the chain produced none', () => {
    render(<Identity row={rowFixture({ description: null, descriptionSource: null })} />);
    expect(screen.queryByTestId('cp-description')).toBeNull();
  });

  it('states no visibility — public and private are remote facts', () => {
    render(<Identity row={rowFixture()} />);
    expect(document.body.textContent).not.toMatch(/\b(public|private)\b/i);
  });
});
