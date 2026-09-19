import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type {
  CompletionCheck,
  CompletionCheckRow,
  CompletionDetail,
  UnknownReason,
} from '../../../generated/protocol';
import { CompletionChecklist } from './CompletionChecklist';
import { CHECK_LABELS, noteFor } from './checklist';

/**
 * **`AC-P3-31-9`** — unknown is not `fail`, is not a dark tick, and has a reason — and
 * **`AC-P3-31-13`'s render half**.
 *
 * The variant set is **enumerated from the generated enum**, never from a literal count
 * (R132/F11): no literal `6` appears anywhere in this file.
 */

afterEach(cleanup);

/** The reasons, read off the generated union rather than counted. */
const REASONS = [
  'needsAccount',
  'notSynced',
  'notRead',
  'notObserved',
  'notRunYet',
  'unreachable',
] as const satisfies readonly UnknownReason[];

const KEYS = [
  'remote',
  'readme',
  'license',
  'description',
  'tests',
  'ci',
  'ciGreen',
  'pushed',
  'deps',
  'release',
] as const satisfies readonly CompletionCheck[];

const row = (over: Partial<CompletionCheckRow> = {}): CompletionCheckRow => ({
  key: 'readme',
  state: 'pass',
  userNa: null,
  unknownReason: null,
  observedAt: 1_700_000_000,
  ...over,
});

const detail = (checks: readonly CompletionCheckRow[]): CompletionDetail => ({
  lit: checks.filter((c) => c.state === 'pass').length,
  evaluable: checks.filter((c) => c.state === 'pass' || c.state === 'fail').length,
  unknown: checks.filter((c) => c.state === 'unknown').length,
  checks,
});

describe('AC-P3-31-9: every unknown reason renders a hollow mark and a note', () => {
  it('covers the whole generated variant set and fails on one with no note', () => {
    let covered = 0;
    for (const reason of REASONS) {
      const { container } = render(
        <CompletionChecklist
          completion={detail([row({ state: 'unknown', unknownReason: reason })])}
        />,
      );
      const item = container.querySelector('.cp-completion-check');
      const mark = container.querySelector<HTMLElement>('.cp-completion-mark');
      const note = container.querySelector('.cp-completion-note');

      expect(item?.getAttribute('data-state')).toBe('unknown');
      expect(mark?.textContent).toBe('◌');
      expect(mark?.getAttribute('data-hollow')).toBe('true');
      // A variant with no note renders nothing here, which is what this assertion catches.
      expect(note?.textContent, `${reason} renders no note`).toBeTruthy();
      expect(note?.textContent).toBe(noteFor(row({ state: 'unknown', unknownReason: reason })));
      covered += 1;
      cleanup();
    }
    console.error(`checklist.ac: covered ${String(covered)} unknown reason variant(s)`);
    expect(covered, 'a run that covered no variant proves nothing').toBeGreaterThan(0);
    expect(covered).toBe(REASONS.length);
  });

  it('never says GITHUB in an unknown note for a locally evaluable check', () => {
    const local: readonly CompletionCheck[] = ['readme', 'license', 'tests', 'ci', 'release'];
    for (const key of local) {
      for (const reason of REASONS) {
        const note = noteFor(row({ key, state: 'unknown', unknownReason: reason }));
        expect(note).not.toContain('GITHUB');
      }
    }
  });

  it('draws unknown as neither lit nor unlit', () => {
    const { container } = render(
      <CompletionChecklist
        completion={detail([
          row({ key: 'readme', state: 'pass' }),
          row({ key: 'license', state: 'fail' }),
          row({ key: 'tests', state: 'unknown', unknownReason: 'notRead' }),
        ])}
      />,
    );
    const glyphs = [...container.querySelectorAll('.cp-completion-mark')].map((m) => m.textContent);
    expect(glyphs).toEqual(['▣', '□', '◌']);
  });
});

describe('AC-P3-31-13: the checklist renders ten rows, or none at all', () => {
  it('renders nothing when the detail is NULL — a Reference project mounts no checklist', () => {
    const { container } = render(<CompletionChecklist completion={null} />);
    expect(container.querySelector('.cp-completion')).toBeNull();
    expect(container.textContent).toBe('');
  });

  it('renders all ten rows with their reasons when evaluable is zero', () => {
    // **Hiding this case would hide the only surface that says why nothing could be scored.**
    const checks = KEYS.map((key) => row({ key, state: 'unknown', unknownReason: 'notRunYet' }));
    const { container } = render(<CompletionChecklist completion={detail(checks)} />);
    const items = [...container.querySelectorAll('.cp-completion-check')];
    expect(items).toHaveLength(10);
    expect(detail(checks).evaluable).toBe(0);
    for (const item of items) {
      expect(item.querySelector('.cp-completion-note')?.textContent).toBe('UNKNOWN · NOT RUN YET');
    }
    // In declaration order, with each label from the one owner.
    expect(items.map((i) => i.querySelector('.cp-completion-label')?.textContent)).toEqual(
      KEYS.map((key) => CHECK_LABELS[key]),
    );
  });
});

describe('the na split renders both forms', () => {
  it('distinguishes a stored ruling from the archetype’s proposal', () => {
    const { container } = render(
      <CompletionChecklist
        completion={detail([
          row({ key: 'tests', state: 'na', userNa: true }),
          row({ key: 'deps', state: 'na', userNa: null }),
        ])}
      />,
    );
    const notes = [...container.querySelectorAll('.cp-completion-note')].map((n) => n.textContent);
    expect(notes).toEqual(['MARKED NOT APPLICABLE', 'NOT APPLICABLE']);
  });
});
