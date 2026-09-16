/**
 * §25.4's display mapping over an **open** vocabulary.
 *
 * `CiRun.conclusion` is a nullable `String` and not an enum: the vocabulary belongs to the forge,
 * and a closed mirror of a third party's vocabulary is R26 by construction — the constraint would
 * one day reject a value its own source emits. So an unrecognised conclusion renders **uppercased
 * and verbatim** rather than as a word this build invented.
 *
 * *"Never `failed`"* is a rule about an **uncomputable check**. A workflow run whose observed
 * conclusion is a failure renders as failed: that is an observation of what the forge did, not an
 * inference about the project.
 *
 * **There is no aggregate.** No "CI green" boolean exists anywhere in phase 2 — the aggregate
 * *is* the check, and the check is phase 3.
 */

/** §25.1, §25.7: at most five runs per repository, and the list renders at most five. */
export const CI_RUN_LIMIT = 5;

/** The label for a stored conclusion. NULL is a run that had not concluded when it was observed. */
export function ciLabel(conclusion: string | null): string {
  if (conclusion === null) return 'RUNNING';
  switch (conclusion) {
    case 'success':
      return 'PASSED';
    case 'failure':
      return 'FAILED';
    case 'cancelled':
      return 'CANCELLED';
    case 'timed_out':
      return 'TIMED OUT';
    case 'action_required':
      return 'ACTION REQUIRED';
    case 'neutral':
      return 'NEUTRAL';
    case 'skipped':
      return 'SKIPPED';
    case 'startup_failure':
      return 'STARTUP FAILURE';
    default:
      return conclusion.toUpperCase();
  }
}

/**
 * The §8.7 token this conclusion takes, as a **token name** and never a literal —
 * `scripts/check-style-tokens.mjs` rejects an undeclared colour anyway, and A13 records that
 * `--pass` and `--fail` are declared tokens rather than values to snap.
 */
export function ciInk(conclusion: string | null): string {
  switch (conclusion) {
    case 'success':
      return '--pass';
    case 'failure':
      return '--fail';
    case 'cancelled':
    case 'timed_out':
    case 'action_required':
      return '--warn';
    default:
      return '--text-3';
  }
}
