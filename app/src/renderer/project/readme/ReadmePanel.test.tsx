import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { ReadmeState } from '../../../generated/protocol';
import { DAY, NOW, rowFixture } from '../testFixtures';
import { README_ABSENT, README_NOT_INDEXED, ReadmePanel } from './ReadmePanel';

afterEach(cleanup);

const draw = (readme: ReadmeState, now = NOW): HTMLElement =>
  render(<ReadmePanel readme={readme} row={rowFixture()} now={now} />).container;

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
describe('AC-P2-25-9 the topic rail renders only when there is a topic', () => {
  const readme: ReadmeState = { state: 'present', text: 'A paragraph.', readAt: NOW - 3600 };

  it('renders one chip for one topic', () => {
    render(<ReadmePanel readme={readme} row={rowFixture()} now={NOW} topics={['rust']} />);
    const rail = screen.getByTestId('cp-readme-topics');
    expect(rail.children).toHaveLength(1);
    expect(rail.textContent).toBe('rust');
  });

  it('renders three chips for three topics', () => {
    render(
      <ReadmePanel readme={readme} row={rowFixture()} now={NOW} topics={['rust', 'cli', 'tui']} />,
    );
    expect(screen.getByTestId('cp-readme-topics').children).toHaveLength(3);
  });

  it('renders no rail element at all for zero topics, asserted as an absence', () => {
    render(<ReadmePanel readme={readme} row={rowFixture()} now={NOW} topics={[]} />);
    expect(screen.queryByTestId('cp-readme-topics')).toBeNull();
  });
});
