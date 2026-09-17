import type { ProjectRow } from '../../generated/protocol.js';
import { languageSigil } from '../derive/languageSigil.js';

export const SUB_LINE_SEPARATOR = ' · ';

const DAY_SECONDS = 86_400;

export interface SubLineInput {
  readonly primaryLanguage: string | null;
  readonly branch: string | null;
  /** Epoch seconds. */
  readonly lastTouchedAt: number;
  readonly inSession: boolean;
  /** Epoch seconds. */
  readonly nowSecs: number;
  /**
   * §23.1's one predicate, `primaryLocation !== null`, and never a second expression of it. The
   * tail is an interaction claim and a project with no copy on this machine has had none.
   *
   * R92: §24.5 fills this seam and adds no field beside it. `hasLocation` would be the same
   * predicate under a second name, over the one expression §23.1 allows it.
   */
  readonly hasWorkingCopy: boolean;
}

/**
 * §24.5's word for a project with no working copy, and the fourth tail word §23 declined to
 * invent while §24 was still open. It states a fact rather than an interaction: for a
 * zero-location project `lastTouchedAt` is the `created_at` fallback — the moment Codotheca wrote
 * the row — so an age here would read `opened this month` about a repository that has never been
 * on this machine.
 */
const NOT_CLONED_TAIL = 'not cloned';

/**
 * Total, since §24.5 supplied the fourth word. It returned `null` for a project with no working
 * copy while the copy for that case was another section's to write; the null branch is gone
 * rather than left behind as a shape nothing produces. `languageSigil` and `branch` keep their
 * own absences — §8.6's line still omits a field it has no value for, and a dash would be a
 * fourth way of saying nothing on one line.
 */
function tail(input: SubLineInput): string {
  if (!input.hasWorkingCopy) return NOT_CLONED_TAIL;
  if (input.inSession) return 'in session';
  const days = Math.max(0, Math.floor((input.nowSecs - input.lastTouchedAt) / DAY_SECONDS));
  if (days <= 30) return 'opened this month';
  return `${String(Math.round(days / 30))} months cold`;
}

/**
 * §8.6's sub-line. It is what separates two similarly named projects, which is why §8.3a added
 * `branch` to the projection for it and why §8.6 rules the whole line off `--text-4`.
 */
export function paletteSubLine(input: SubLineInput): string {
  const fields: string[] = [];
  const sigil = languageSigil(input.primaryLanguage);
  if (sigil !== null) fields.push(sigil);
  if (input.branch !== null && input.branch !== '') fields.push(input.branch);
  fields.push(tail(input));
  return fields.join(SUB_LINE_SEPARATOR);
}

export function subLineInputFor(
  row: ProjectRow,
  inSession: boolean,
  nowSecs: number,
): SubLineInput {
  return {
    primaryLanguage: row.primaryLanguage,
    branch: row.branch,
    lastTouchedAt: row.lastTouchedAt,
    inSession,
    nowSecs,
    hasWorkingCopy: row.primaryLocation !== null,
  };
}
