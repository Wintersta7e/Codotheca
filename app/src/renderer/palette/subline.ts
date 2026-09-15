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
   */
  readonly hasWorkingCopy: boolean;
}

/**
 * `null` means *the field is not computed*, not *render an em dash*: §8.6's line omits a field
 * it has no value for, and a dash here would be a fourth way of saying nothing on one line.
 *
 * For a zero-location project `lastTouchedAt` is the `created_at` fallback — the moment
 * Codotheca wrote the row — so this tail would otherwise read `opened this month` about a
 * repository that has never been on this machine. Where §24 supplies a not-cloned row copy the
 * tail takes it; §23 invents no fourth tail word.
 */
function tail(input: SubLineInput): string | null {
  if (!input.hasWorkingCopy) return null;
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
  const suffix = tail(input);
  if (suffix !== null) fields.push(suffix);
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
