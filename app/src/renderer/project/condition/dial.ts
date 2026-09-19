/**
 * §33.7's two-clock dial: where each needle points, and what the divergence between them reads.
 *
 * **Two needles side by side is not a merged ledger and not a stacked bar.** The divergence is
 * the whole point, because *finished* and *neglected* differ precisely in whether the two clocks
 * agree — the interaction clock (`condition_signal`) and the commit clock
 * (`condition_material`).
 *
 * **§33 restates no fill, no ring and no day edge.** `conditionDot` in `derive/condition.ts` is
 * §5.4a's one owner in code and already answers all seven variants plus the `is_reference` and
 * `is_archived` overrides; a second copy of that table is the defect criterion 58 exists to
 * catch.
 */
import type { ConditionSignal } from '../../../generated/protocol';

/**
 * §5.4's ladder: `live` `idle` `dormant` `neglected` `abandoned`, in that order, warmest first.
 *
 * `offline` and `empty` are **off the ladder** — `null`, not a sixth and seventh position. An
 * offline copy is a reading nobody could take and an empty repository has no history to measure,
 * and neither is a colder rung of the same scale.
 */
export const LADDER_POSITION: Readonly<Record<ConditionSignal, number | null>> = Object.freeze({
  live: 0,
  idle: 1,
  dormant: 2,
  neglected: 3,
  abandoned: 4,
  offline: null,
  empty: null,
});

/**
 * The prototype's dial is a **full circle**, so the five rungs take the centres of five equal
 * sectors: `index × 72 + 36`.
 */
export const ROSE_SECTOR_DEG = 72;

/**
 * Degrees clockwise from the top. **An off-ladder value takes `0°`**, which is a sector
 * *boundary* and therefore no rung's centre — so no off-ladder needle ever coincides with a
 * rung, which is what stops the dial claiming a reading it does not have. §5.4a's own treatment
 * carries the rest: `offline`'s bright ring, `empty`'s dashed one.
 */
export function needleAngle(signal: ConditionSignal): number {
  const position = LADDER_POSITION[signal];
  if (position === null) return 0;
  return position * ROSE_SECTOR_DEG + ROSE_SECTOR_DEG / 2;
}

export type Divergence = 'IN STEP' | 'LIT BUT DUSTY' | 'CLEAN BUT DARK';

/**
 * A three-way comparison of the two values' **positions on §5.4's ladder**.
 *
 * `null` when the material clock was never computed — §33.7 draws no inner needle and no
 * divergence line for that, never a needle at zero — and `null` when either value is off the
 * ladder, because there is no distance between a rung and something that is not on the scale.
 *
 * **The prototype's percentage-distance test is superseded.** It read
 * `Math.abs(glowPct - matPct) < 12 ? 'IN STEP' : …`; a 12-point window over a percentage is a
 * sixth threshold nothing else in the product carries, and it would report two adjacent rungs as
 * the same reading.
 */
export function divergence(
  signal: ConditionSignal,
  material: ConditionSignal | null,
): Divergence | null {
  if (material === null) return null;
  const lit = LADDER_POSITION[signal];
  const clean = LADDER_POSITION[material];
  if (lit === null || clean === null) return null;
  if (lit === clean) return 'IN STEP';
  // A lower position is warmer. Touched recently but not committed to reads LIT BUT DUSTY;
  // committed to but never opened reads CLEAN BUT DARK.
  return lit < clean ? 'LIT BUT DUSTY' : 'CLEAN BUT DARK';
}
