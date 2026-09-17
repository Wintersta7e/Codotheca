/**
 * §24.4's pacing: the animation is **not** the readout.
 *
 * §10.3a ruled the identical shape for the first-run beam — tying a decorative element to real
 * progress reinstates the bar §10.2 forbids. So the sequence has a floor and it **compresses**;
 * it never skips a stage. A clone that finishes in 300 ms must not flash six stages, and a user
 * who saw `receiving` appear and vanish in one frame learns nothing except that something
 * flickered.
 */
import type { InstallStage, InstallStageKind } from '../../generated/protocol.js';

/** The shortest the whole sequence may take, in milliseconds. */
export const STAGE_FLOOR_MS = 900;

/** §24.4's order. The pacer walks this and never reorders it. */
export const STAGE_ORDER: readonly InstallStageKind[] = [
  'plans',
  'enumerating',
  'receiving',
  'assembling',
  'cladding',
  'settled',
];

/**
 * Which stages may be shown after `elapsedMs` of a run whose observed stages are `observed`.
 *
 * **Compresses, never skips.** The result is a prefix of the stages actually observed, so a stage
 * the core never reported is never rendered — and every stage that *was* reported eventually is,
 * because the prefix only grows. A run that ends before the floor keeps revealing its remaining
 * stages as time passes rather than dropping them.
 *
 * The pacer is a pure function of `(observed, elapsedMs)`: it holds no timer and no state, which
 * is what makes "never skips" a property a test can check over any transcript at all.
 */
export function pacedStages(
  observed: readonly InstallStage[],
  elapsedMs: number,
): readonly InstallStageKind[] {
  if (observed.length === 0) return [];
  // The first stage is on screen at t=0 and the last becomes reachable **exactly** at the floor,
  // so the gaps are divided by `length - 1`, not by `length`. Dividing by `length` puts the final
  // stage one slice early — measured: with six stages it completed at 899 ms against a 900 ms
  // floor, which is the whole thing this constant exists to prevent.
  if (observed.length === 1) return [observed[0]!.stage];
  const perStage = STAGE_FLOOR_MS / (observed.length - 1);
  const earned = Math.floor(Math.max(elapsedMs, 0) / perStage) + 1;
  const shown = Math.min(observed.length, Math.max(1, earned));
  return observed.slice(0, shown).map((stage) => stage.stage);
}

/**
 * The figure for one stage: `<done> of <total>` where a denominator exists, a bare count where it
 * does not, and nothing at all where there is no count.
 *
 * **There is no percentage branch, and no arithmetic that could produce one.** `InstallStage`
 * carries no aggregate field, so there is nothing here to divide.
 */
export function stageFigure(stage: InstallStage): string | null {
  if (stage.done === null) return null;
  const done = stage.done.toLocaleString('en-US');
  if (stage.total === null) return done;
  return `${done} of ${stage.total.toLocaleString('en-US')}`;
}

/** `2.14 MB`, or `null` when git reported no byte count for this phase. */
export function stageBytes(stage: InstallStage): string | null {
  if (stage.bytes === null) return null;
  const mib = stage.bytes / (1024 * 1024);
  if (mib >= 0.1) return `${Math.round(mib * 10) / 10} MB`;
  return `${Math.round(stage.bytes / 1024)} KB`;
}
