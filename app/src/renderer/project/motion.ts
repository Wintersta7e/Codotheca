/**
 * §8.5.1's beats, exported because the entry is one gesture across two owners: the shelf runs
 * the tile collapse, the recede and the beam; this page runs the power-on, the rail slide and
 * the right-column cascade. A duration written in both places is one that drifts.
 *
 * §11.6 governs whether any of it runs at all: at `reduced` and `off` the page paints in its
 * final state and emits no entry frame. That gate is in the stylesheet, not here, so the
 * numbers survive a tier change unread.
 */
export const PAGE_ENTRY = {
  /** The shelf swaps the view here; the page's own power-on starts at the same instant. */
  viewSwapMs: 620,
  powerOnMs: 620,
  railMs: 400,
  riseMs: 420,
  cascadeStepMs: 80,
  cascadeStartMs: 100,
} as const;

export const PAGE_EXIT = {
  rackOutMs: 340,
  shelfRestoredMs: 300,
  landingClearedMs: 1600,
} as const;

/**
 * The cascade the right column runs on: §8.5.1 gives an 80 ms step from `.1s`. Returned as a
 * string because it is written straight into a custom property, and a caller that formats it
 * itself is a second place the unit can be got wrong.
 */
export function cascadeDelay(step: number): string {
  const index = Math.max(0, Math.floor(step));
  return `${String(PAGE_ENTRY.cascadeStartMs + index * PAGE_ENTRY.cascadeStepMs)}ms`;
}
