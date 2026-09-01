/**
 * §10.5a's 2 px indeterminate line, and §10.6's four rescan triggers surfaced.
 *
 * Indeterminate because a generation walk has no denominator until it ends — the reason §10.2
 * already refuses a percentage — and a bar that retreats poisons every figure beside it.
 */
import type { ReactElement } from 'react';
import { allowsTravellingHighlights } from '../motion/tier';
import type { ResolvedTier } from '../motion/tier';

/** §10.6's four mechanisms, in the order that section lists them. */
export type RescanTrigger = 'launch' | 'focus' | 'watcher' | 'manual';

/**
 * §10.5a: drawn only if the walk is still running at ~400 ms.
 *
 * The walk measured 101,154 dirs/s and criterion 18 puts an all-fingerprints-unchanged pass
 * under 5 s for a thousand repositories, so the warm case finishes below this and the user
 * correctly sees nothing at all.
 */
export const RESCAN_ARM_AFTER_MS = 400;

/**
 * §10.5a: 1.15 s `linear`, looped. This is the section strip-light geometry with its easing
 * removed — `cubic-bezier(.32,.6,.3,1)` is a one-shot curve and reads as a stutter when looped.
 */
export const RESCAN_TRAVEL_MS = 1150;

export const RESCAN_LINE_LABEL = 'RESCANNING · OPEN THE SCAN SUMMARY';

/**
 * Whether the line is drawn at all.
 *
 * `focus` is excluded outright: §10.6 mechanism 2 runs J2 over visible tiles only, and an
 * indicator on every alt-tab is a scheduled animation at idle under another name (criterion 21).
 */
export function armsIndicator(
  trigger: RescanTrigger,
  running: boolean,
  elapsedMs: number,
): boolean {
  if (trigger === 'focus') return false;
  return running && elapsedMs >= RESCAN_ARM_AFTER_MS;
}

export interface RescanLineProps {
  readonly trigger: RescanTrigger;
  readonly running: boolean;
  /** Milliseconds since the walk started, from the host's monotonic source (R3). */
  readonly elapsedMs: number;
  readonly tier: ResolvedTier;
  /** §11.1's scan summary, which already promises reachability "from the scan line". */
  readonly onOpenSummary: () => void;
}

export function RescanLine(props: RescanLineProps): ReactElement | null {
  if (!armsIndicator(props.trigger, props.running, props.elapsedMs)) {
    // Not a paused line and not a zero-height one: nothing, so idle holds zero frames.
    return null;
  }
  // §11.6's clamp lives in the class rather than in a tier selector: this line is full-bleed
  // under the top bar and has no `.cdt-fr-view` ancestor for a stylesheet to reach it through.
  const travelling = allowsTravellingHighlights(props.tier);
  return (
    <button
      type="button"
      className={
        travelling ? 'cdt-fr-rescan-line cdt-fr-rescan-line--travelling' : 'cdt-fr-rescan-line'
      }
      aria-label={RESCAN_LINE_LABEL}
      onClick={() => {
        props.onOpenSummary();
      }}
    />
  );
}
