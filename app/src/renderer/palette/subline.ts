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
}

function tail(input: SubLineInput): string {
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
  };
}
