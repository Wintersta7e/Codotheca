/**
 * [p3] §30.3 and §30.4 — **the four outcome forms, and the two causes of `off`.**
 *
 * `off` and `unknown` are stored in different fields and rendered in different words, and
 * **neither may be rendered as the other**: one is a user act, the other is the app's own
 * failure. Four forms are required — `ok`, `off`, `notApplicable`, `unknown` — and **none of the
 * latter three may be the passing form.**
 *
 * The words `clean`, `healthy`, `none` and `all` appear in **no rendered string, accessible name
 * or CSS class** in this feature (R130/F9, scoped to renderings). A wire value named `clean` is
 * permitted and is never rendered as that word. `app/src/renderer/derive/observation.ts` already
 * holds the same ban for §6 and is the shape this copies.
 *
 * Which colours, sizes and shapes the three non-passing states take is §33's, against §8.7's
 * contrast floors. §30 rules only that they are distinct and that neither `off` nor `unknown` is
 * the passing form.
 */
import type {
  CheckOutcome,
  DebtSource,
  HealthBasis,
  HealthCheck,
  HealthState,
  Settings,
  UnknownReason,
} from '../../../generated/protocol';

/**
 * What separates the parts of one row on the `HEALTH` tab, **in the text itself**, so a row
 * reads as parts with or without a stylesheet — `missing_readme · OPEN`, never
 * `missing_readmeOPEN`.
 */
export const PART_SEPARATOR = ' · ';

/** The rendered form of one check: a word, and the sentence under it. */
export interface CheckForm {
  readonly outcome: CheckOutcome;
  readonly word: string;
  readonly detail: string;
}

/**
 * §30.3's vocabulary, rendered. Each reason names **a different act the user is owed** —
 * `notRunYet` and `unreachable` are `notObserved`'s two self-resolving neighbours and say so.
 */
const REASON_SENTENCE: Record<UnknownReason, string> = {
  needsAccount: 'needs an account to read from',
  notSynced: 'not read back from its source yet',
  notRead: 'could not be read — try again, or raise the budget',
  notObserved: 'there is nothing here to observe',
  notRunYet: 'scheduled, and has not run yet',
  unreachable: 'the store it reads cannot be reached',
};

/** The four words. **None of the last three is the passing one**, which is the whole rule. */
const OUTCOME_WORD: Record<CheckOutcome, string> = {
  ok: 'PASSED',
  failed: 'OPEN',
  unknown: 'UNKNOWN',
  off: 'SWITCHED OFF',
  notApplicable: 'DOES NOT APPLY',
};

/**
 * The form one check takes.
 *
 * An `unknown` check **is always named**, with §30.3's vocabulary: a count of unknowns with no
 * reason is the prototype's single `UNKNOWN · NEEDS GITHUB` note in a different shape.
 */
export function formFor(check: HealthCheck): CheckForm {
  const word = OUTCOME_WORD[check.outcome];
  if (check.outcome === 'unknown') {
    // The reason is never omitted, defaulted or collapsed to one string. A payload that carried
    // none would be a producer defect, and saying so is more honest than inventing a sentence.
    const reason = check.unknownReason;
    return {
      outcome: check.outcome,
      word,
      detail: reason === null ? 'no reason was recorded' : REASON_SENTENCE[reason],
    };
  }
  return { outcome: check.outcome, word, detail: '' };
}

/** §30.9's two causes of `off`, which are two different sentences (R142). */
export type OffCause = 'switchedOff' | 'grantMissing';

/**
 * Which of the two put this check in `off`.
 *
 * **The switch wins when both would apply**, because it is the user's explicit act about this
 * check and offering a grant for a check they have turned off is a control that changes nothing.
 * An implementation that renders one string for both causes has built the ask and made it
 * unreachable — and no test around it would fail, which is why this is one named function.
 *
 * The renderer tells them apart from **two settings fields it already holds**, not from a
 * seventh `UnknownReason` and not from a new wire field.
 */
export function offCause(check: HealthCheck, settings: Settings): OffCause {
  const entry = settings.healthChecks.find((s) => s.check === check.id);
  if (entry !== undefined && !entry.enabled) return 'switchedOff';
  if (needsSourceGrant(check.id) && !settings.contentScanEnabled) return 'grantMissing';
  return 'switchedOff';
}

/**
 * The sources whose evidence needs §29.8's source-reading grant.
 *
 * `todo_marker` alone: lockfiles and the four presence predicates ride J6's existing named-file
 * grant, and *"we read your source code"* and *"we read your `package-lock.json`"* are different
 * sentences to a user.
 */
export function needsSourceGrant(source: DebtSource): boolean {
  return source === 'todo_marker';
}

/**
 * §30.4's basis line, or `null` when there is nothing honest to say.
 *
 * **`ran = 0` on a `live` reading renders no count at all — the basis alone.** A `0` beside a
 * denominator of zero conveys nothing and is the bare zero this section exists to prevent; the
 * case is real and ordinary, because a project between enrolment and its first sweep has every
 * check `unknown` with reason `notRunYet`.
 *
 * A `null` basis is *nothing has been observed*, which the per-check list already says one row at
 * a time. Inventing a coverage figure for it would be the same zero one step along.
 */
export function basisLine(basis: HealthBasis | null, state: HealthState): string | null {
  if (basis === null) return null;
  if (state !== 'live' && state !== 'frozen') return null;
  const parts: string[] = [];
  if (basis.ran > 0) parts.push(`${basis.ran} of ${basis.eligible} checks ran`);
  else parts.push(`no check has run yet, of ${basis.eligible} that could`);
  if (basis.unknown > 0) parts.push(`${basis.unknown} could not be evaluated`);
  if (basis.off > 0) parts.push(`${basis.off} switched off`);
  if (basis.notApplicable > 0) parts.push(`${basis.notApplicable} do not apply here`);
  return parts.join(' · ');
}

/**
 * §30.4's *nothing outstanding* gate, stated once and consumed here.
 *
 * **`scoredOpen = 0` reads as *nothing open* only when `unknown = 0`.** With any unknown it is
 * *nothing open in the checks that ran*, and saying more would be a claim of currency the
 * producer does not have.
 */
export function openLine(scoredOpen: number | null, basis: HealthBasis | null): string | null {
  if (scoredOpen === null || basis === null) return null;
  if (basis.ran === 0) return null;
  if (scoredOpen > 0) return `${scoredOpen} open`;
  return basis.unknown === 0 ? 'nothing open' : 'nothing open in the checks that ran';
}
