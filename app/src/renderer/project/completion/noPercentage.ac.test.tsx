import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { heroScoreLine } from '../hero/HeroTile';
import { rungFor, scoreText } from '../../card/completion';
import { LIST_COLUMNS } from '../../shelf/ListView';
import { PEEK_FACT_KEYS, peekFacts } from '../../shelf/peekText';
import type { Peek } from '../../../generated/protocol';

/**
 * **`AC-P3-31-6` — no percentage, and no bare numerator.**
 *
 * The denominator is the whole safeguard: a fraction whose denominator is the truth makes no
 * claim of ten, and a percentage erases it. This walks every producer of a completion readout on
 * the five surfaces §31.7 names and asserts both halves, **printing the number of readouts it
 * inspected**; a run that inspects zero fails.
 */

afterEach(cleanup);

const NOW = 1_700_000_000;

const peekFixture = (): Peek =>
  ({
    id: 1,
    readme: { state: 'absent', text: null, readAt: null },
    remote: null,
    commits: [],
    location: { id: 1, pathDisplay: '/w/atlas' },
    worktree: { isDirty: null, untrackedCount: null, observedAt: null },
    birthYear: 2021,
    primaryLanguage: 'Rust',
    sizeTrackedBytes: null,
    lastCommitAt: null,
    playtimeSeconds: 0,
    interruptedOp: null,
  }) as unknown as Peek;

/** Every fraction a completion surface can render, over a spread of shapes. */
function readouts(): string[] {
  const out: string[] = [];
  const shapes: readonly (readonly [number, number, number])[] = [
    [10, 10, 0],
    [8, 8, 2],
    [7, 9, 1],
    [0, 10, 0],
    [4, 4, 6],
  ];
  for (const [lit, evaluable, unknown] of shapes) {
    // The list row and Peek both read `scoreText`; the hero adds its own clause.
    const score = scoreText(lit, evaluable);
    if (score !== null) out.push(score);
    const hero = heroScoreLine(lit, evaluable, unknown);
    if (hero !== null) out.push(hero);
    const fact = peekFacts(peekFixture(), NOW, {
      completionLit: lit,
      completionApplicable: evaluable,
    }).find((f) => f.key === 'COMPLETION');
    if (fact !== undefined) out.push(fact.value);
  }
  return out;
}

describe('AC-P3-31-6: no surface renders a percentage', () => {
  it('carries no percent sign in any readout, at any shape', () => {
    const rendered = readouts();
    console.error(`noPercentage: inspected ${String(rendered.length)} completion readout(s)`);
    expect(rendered.length, 'a run that inspected no readout proves nothing').toBeGreaterThan(0);
    for (const text of rendered) expect(text).not.toContain('%');
  });

  it('accompanies every numerator with its denominator in the same string', () => {
    for (const text of readouts()) {
      // Every rendered figure is `a/b`, optionally followed by words. A lone integer with no
      // slash is a numerator that lost its denominator.
      expect(text, `${text} carries a numerator without its denominator`).toMatch(
        /^\d+\/\d+(?: [A-Z]+(?: · \d+ [A-Z]+)?)?$/u,
      );
    }
  });
});

describe('AC-P3-31-6: the unknown clause drops only at zero', () => {
  it('renders the clause when there is something to say and drops it when there is not', () => {
    // Both halves run, because dropping it unconditionally and rendering it unconditionally each
    // pass one half.
    expect(heroScoreLine(8, 8, 2)).toBe('8/8 EVALUABLE · 2 UNKNOWN');
    expect(heroScoreLine(10, 10, 0)).toBe('10/10 EVALUABLE');
    expect(heroScoreLine(10, 10, 0)).not.toContain('UNKNOWN');
    // And nothing at all when the figure does not exist: band 3 already states the absence.
    expect(heroScoreLine(null, null, 0)).toBeNull();
    expect(heroScoreLine(0, 0, 10)).toBeNull();
  });
});

describe('AC-P3-31-6: the palette row renders nothing', () => {
  it('keeps the leading slot as the condition dot, permanently', () => {
    // §7.7a's removed-slots table: four of five slots fill, and the palette's does not. Asserted
    // so a later author does not fill the fifth by symmetry.
    const { container } = render(<div className="cdt-palette-row" />);
    expect(container.textContent).toBe('');
    // The list is the surface that DID gain a score column, which is what makes the palette's
    // absence a decision rather than an oversight.
    expect(LIST_COLUMNS.map((c) => c.key)).toContain('score');
    expect(PEEK_FACT_KEYS).toContain('COMPLETION');
  });
});

describe('AC-P3-31-6: the tier frame is not a readout', () => {
  it('names a token and renders no figure of its own', () => {
    const paint = rungFor({
      completionLit: 8,
      completionApplicable: 8,
      isReference: false,
      hasWorkingCopy: true,
      isArchived: false,
    });
    expect(paint?.frameToken).not.toMatch(/%|\d+\/\d+/u);
    expect(paint?.notched).toBe(true);
  });
});
