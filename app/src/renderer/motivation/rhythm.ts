/**
 * §43.2's `RHYTHM` over §39's `RhythmState`: the line, qualified while the git history it counts
 * is still being read, and one cell per `RhythmDay`, Monday first. A `waived` day is its own cell
 * and never drawn as `inactive`; `notComputed` is the `--unknown` fill.
 */
import type { RhythmCoverage, RhythmDay, RhythmState } from '../../generated/protocol';
import { DAY_LETTERS, DAY_NAMES, RHYTHM_DAY_WORDS, RHYTHM_LINE, RHYTHM_PARTIAL } from './copy';

export interface RhythmCell {
  /** Drawn by `band.css` as `[data-day]`, one rule per variant. */
  readonly day: RhythmDay;
  readonly letter: string;
  /** The day and its state in words, the cell's accessible name. */
  readonly name: string;
}

export interface RhythmModel {
  readonly line: string;
  /** The qualifier while `gitCoverage` is `partial`; `null` once the history is read. */
  readonly partial: string | null;
  readonly cells: readonly RhythmCell[];
}

/** A variant the schema added after this table: fail rather than draw it blank. */
function noRendering(variant: never): never {
  throw new Error(`the rhythm has no rendering for ${String(variant)}`);
}

function dayWords(day: RhythmDay): string {
  switch (day) {
    case 'active':
    case 'inactive':
    case 'pending':
    case 'future':
    case 'waived':
    case 'notComputed':
      return RHYTHM_DAY_WORDS[day];
    default:
      return noRendering(day);
  }
}

function qualifier(coverage: RhythmCoverage): string | null {
  switch (coverage) {
    case 'complete':
      return null;
    case 'partial':
      return RHYTHM_PARTIAL;
    default:
      return noRendering(coverage);
  }
}

/**
 * The week's active-day count, or `null` while any of its days is `notComputed`: a count over a
 * week the core has not finished reading is unknown, and unknown is never drawn as a number.
 */
export function activeDaysFigure(rhythm: RhythmState): number | null {
  return rhythm.days.includes('notComputed') ? null : rhythm.activeDays;
}

export function rhythmModel(rhythm: RhythmState): RhythmModel {
  return {
    line: RHYTHM_LINE(activeDaysFigure(rhythm), rhythm.target, rhythm.bestWeeks),
    partial: qualifier(rhythm.gitCoverage),
    cells: rhythm.days.map((day, index) => ({
      day,
      letter: DAY_LETTERS[index] ?? '',
      name: `${DAY_NAMES[index] ?? ''}, ${dayWords(day)}`,
    })),
  };
}
