import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { Peek } from '../../generated/protocol.js';
import { allowsTransforms } from '../motion/tier.js';
import { peekFacts } from './peekText.js';
import { PEEK_ENTER_CLASS, PeekPanel } from './Peek.js';

afterEach(cleanup);

const NOW = Math.floor(Date.UTC(2026, 5, 15, 14, 30) / 1000);
const peek = (over: Record<string, unknown> = {}): Peek =>
  ({
    id: 1,
    readme: {
      state: 'present',
      text: '# Title\n<script>alert(1)</script>\n\nsecond',
      readAt: null,
    },
    commits: [
      {
        sha: '9f1c2b7a4d',
        subject: 'tighten the walk',
        at: Date.UTC(2026, 2, 4) / 1000,
        tzOffsetMin: 0,
      },
    ],
    location: { id: 7, pathDisplay: 'D:\\work\\atlas' },
    worktree: { observedAt: NOW - 120, isDirty: false, untrackedCount: 0 },
    birthYear: 2019,
    primaryLanguage: 'Rust',
    sizeTrackedBytes: null,
    lastCommitAt: null,
    playtimeSeconds: 0,
    interruptedOp: null,
    ...over,
  }) as unknown as Peek;

describe('PeekPanel', () => {
  it('renders README markup as literal text', () => {
    const { container } = render(<PeekPanel peek={peek()} now={NOW} tier="full" />);

    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('.cdt-peek-readme')?.textContent).toBe(
      '# Title <script>alert(1)</script>',
    );
  });

  it('names the not-indexed README state apart', () => {
    render(
      <PeekPanel
        peek={peek({ readme: { state: 'not_indexed', text: null, readAt: null } })}
        now={NOW}
        tier="full"
      />,
    );

    expect(screen.getByText('No README indexed yet.')).toBeTruthy();
  });

  it('renders exactly five facts and no completion fact', () => {
    const { container } = render(<PeekPanel peek={peek()} now={NOW} tier="full" />);
    const keys = [...container.querySelectorAll('.cdt-peek-fact-key')].map(
      (element) => element.textContent,
    );

    expect(keys).toEqual(['BIRTH', 'LANGUAGE', 'TRACKED', 'LAST COMMIT', 'PLAYTIME']);
    expect(container.textContent).not.toContain('COMPLETION');
  });

  it('reports no changes as of the observation and never says clean', () => {
    const { container } = render(<PeekPanel peek={peek()} now={NOW} tier="full" />);
    const observation = container.querySelector('.cdt-peek-observation')?.textContent;

    expect(observation).toContain('no changes as of');
    expect(observation?.toLowerCase()).not.toContain('clean');
  });

  it('renders an interrupted operation as a chip, never as a sentence', () => {
    const { container } = render(
      <PeekPanel peek={peek({ interruptedOp: 'merge' })} now={NOW} tier="full" />,
    );

    expect(container.querySelector('[data-chip="interrupted"]')?.textContent).toBe('INTERRUPTED');
    expect(container.textContent).not.toMatch(/is open here and unfinished/);
  });

  it('carries no roast on any triage input', () => {
    const { container } = render(
      <PeekPanel
        peek={peek({
          interruptedOp: 'merge',
          worktree: { observedAt: NOW, isDirty: true, untrackedCount: 2 },
        })}
        now={NOW}
        tier="full"
      />,
    );

    expect(container.querySelector('.cdt-roast')).toBeNull();
    for (const phrase of ['Nothing pushes a stash', 'Uncommitted work here', 'not the copy']) {
      expect(container.textContent).not.toContain(phrase);
    }
  });

  it('draws no rank and no completion-not-computed sentence', () => {
    const { container } = render(<PeekPanel peek={peek()} now={NOW} tier="full" />);

    expect(container.querySelector('.cdt-rank')).toBeNull();
    expect(container.textContent).not.toContain('Completion not computed');
  });

  it('dates each commit beside its short SHA', () => {
    const { container } = render(<PeekPanel peek={peek()} now={NOW} tier="full" />);
    const commit = container.querySelector('.cdt-peek-commit');

    expect(commit?.textContent).toContain('9f1c2b7');
    expect(commit?.textContent).toContain('2026-03-04');
  });

  it('renders the path but no link the renderer could act on', () => {
    const { container } = render(<PeekPanel peek={peek()} now={NOW} tier="full" />);

    expect(container.querySelector('.cdt-peek-path')?.textContent).toBe('D:\\work\\atlas');
    expect(container.querySelector('a')).toBeNull();
  });

  it('does not add the enter class when effects are off', () => {
    const { container } = render(<PeekPanel peek={peek()} now={NOW} tier="off" />);

    expect(container.querySelector(`.${PEEK_ENTER_CLASS}`)).toBeNull();
  });

  it('renders only its header while the payload is in flight', () => {
    const { container } = render(<PeekPanel peek={null} now={NOW} tier="full" />);

    expect(screen.getByText('PEEK')).toBeTruthy();
    expect(container.querySelector('.cdt-peek-facts')).toBeNull();
    expect(container.textContent).not.toMatch(/0|\u2014/);
  });

  it('follows the transform predicate for reduced and full tiers', () => {
    const { container, rerender } = render(<PeekPanel peek={peek()} now={NOW} tier="reduced" />);

    expect(container.querySelector('.cdt-peek')?.classList.contains(PEEK_ENTER_CLASS)).toBe(
      allowsTransforms('reduced'),
    );

    rerender(<PeekPanel peek={peek()} now={NOW} tier="full" />);

    expect(container.querySelector('.cdt-peek')?.classList.contains(PEEK_ENTER_CLASS)).toBe(
      allowsTransforms('full'),
    );
  });

  it('renders every fact value returned by peekFacts', () => {
    const thePeek = peek({
      birthYear: 2020,
      primaryLanguage: 'TypeScript',
      sizeTrackedBytes: 2048,
      lastCommitAt: NOW - 3600,
      playtimeSeconds: 7620,
    });
    const { container } = render(<PeekPanel peek={thePeek} now={NOW} tier="full" />);
    const values = [...container.querySelectorAll('.cdt-peek-fact-value')].map(
      (element) => element.textContent,
    );

    expect(values).toEqual(peekFacts(thePeek, NOW).map((fact) => fact.value));
  });

  it('renders at most the first three commits', () => {
    const commits = [
      { sha: '1111111aaa', subject: 'one', at: NOW, tzOffsetMin: 0 },
      { sha: '2222222bbb', subject: 'two', at: NOW, tzOffsetMin: 0 },
      { sha: '3333333ccc', subject: 'three', at: NOW, tzOffsetMin: 0 },
      { sha: '4444444ddd', subject: 'four', at: NOW, tzOffsetMin: 0 },
      { sha: '5555555eee', subject: 'five', at: NOW, tzOffsetMin: 0 },
    ];
    const { container } = render(<PeekPanel peek={peek({ commits })} now={NOW} tier="full" />);
    const rows = [...container.querySelectorAll('.cdt-peek-commit')];

    expect(rows).toHaveLength(3);
    expect(rows.map((row) => row.querySelector('.cdt-peek-sha')?.textContent)).toEqual([
      '1111111',
      '2222222',
      '3333333',
    ]);
  });

  it('omits the observation paragraph when the observation is absent', () => {
    const { container } = render(
      <PeekPanel
        peek={peek({
          worktree: { observedAt: null, isDirty: null, untrackedCount: null },
        })}
        now={NOW}
        tier="full"
      />,
    );

    expect(container.querySelector('.cdt-peek-observation')).toBeNull();
  });
});
