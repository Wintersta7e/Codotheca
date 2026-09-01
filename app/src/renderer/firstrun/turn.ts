/**
 * §10.4a's turn: one line, one button, one way out.
 *
 * The line is a ladder rather than a single sentence because the design's hard-coded
 * `You have <n> projects with unpushed work.` closes a tidy library's first run on `0`.
 *
 * R12: neither qualifier string is written here. `copy.ts` owns every string the first-run
 * screens render, and a second spelling of a sentence about honesty is the drift this project
 * has paid for four times.
 */
import { formatClock } from '../derive/observation';
import { FETCH_QUALIFIER, OBSERVED_QUALIFIER } from './copy';

export type TurnRung = 1 | 2 | 3 | 4;

/** The three counts the ladder is tested against, plus the total rung 4 states. */
export interface TurnCounts {
  readonly unpushed: number;
  readonly dirty: number;
  readonly interrupted: number;
  readonly total: number;
}

export interface TurnModel {
  readonly rung: TurnRung;
  readonly count: number;
  readonly line: string;
  /** The claim's honesty qualifier, or null where the rung claims nothing that needs one. */
  readonly qualifier: string | null;
  /** What `SHOW ME` lands on. Empty on rung 4. */
  readonly query: string;
}

export const TURN_QUERIES: Readonly<Record<TurnRung, string>> = {
  1: 'is:unpushed',
  2: 'is:dirty',
  3: 'is:interrupted',
  4: '',
};

/** §6: worktree state is not cacheable, so a worktree claim names when it was observed. */
export function worktreeQualifier(observedAtSecs: number): string {
  return OBSERVED_QUALIFIER(formatClock(observedAtSecs));
}

export function turnLine(rung: TurnRung, n: number): string {
  const noun = n === 1 ? 'project' : 'projects';
  switch (rung) {
    case 1:
      return `You have ${String(n)} ${noun} with unpushed work.`;
    case 2:
      return `You have ${String(n)} ${noun} with uncommitted changes.`;
    case 3:
      return `You have ${String(n)} ${noun} with a merge or rebase left half-finished.`;
    case 4:
      return `${String(n)} ${noun}, most recently touched first.`;
  }
}

function rungModel(rung: TurnRung, count: number, qualifier: string | null): TurnModel {
  return { rung, count, line: turnLine(rung, count), qualifier, query: TURN_QUERIES[rung] };
}

/**
 * The first rung with a count of at least one, with rung 4 unconditional beneath them.
 *
 * `worktreeObservedAt` is the newest worktree observation across the counted projects. When it
 * is null, rungs 2 and 3 are skipped: a count of dirty or interrupted repositories with no
 * observation time behind it is exactly the "presented as current without its observation time"
 * that criterion 23 forbids, and there is a truthful rung below it.
 */
export function turnModel(counts: TurnCounts, worktreeObservedAt: number | null): TurnModel {
  if (counts.unpushed >= 1) {
    return rungModel(1, counts.unpushed, FETCH_QUALIFIER);
  }
  if (worktreeObservedAt !== null) {
    const qualifier = worktreeQualifier(worktreeObservedAt);
    if (counts.dirty >= 1) {
      return rungModel(2, counts.dirty, qualifier);
    }
    if (counts.interrupted >= 1) {
      return rungModel(3, counts.interrupted, qualifier);
    }
  }
  return rungModel(4, counts.total, null);
}
