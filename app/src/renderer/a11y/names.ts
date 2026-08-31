import type { ConditionSignal } from '../../generated/protocol';

/**
 * §11.7's accessible names. What an 8px dot *means* is a product decision a developer cannot
 * invent, so it is spec content and it lives here — one module, so no surface can name the same
 * mark twice and differently.
 *
 * There is deliberately **no repository-badge name**: §7.7 rules all four shields out of phase 1,
 * so that indicator never mounts and naming it would be a dead switch one layer down.
 */

/**
 * A gridcell may contain interactive descendants; an `option` may not, and the pin is a real
 * `<button>` inside the card. `gridcell` also carries `aria-selected` and a roving `tabindex`
 * natively, which is what §11.7 asks for.
 */
export const CARD_ROLE = 'gridcell';
export const GRID_ROLE = 'grid';
export const GRID_ROW_ROLE = 'row';

/** §11.7: the scan count is announced at milestones, not on every repaint. */
export const SCAN_COUNT_ROLE = 'status';

/** §11.7: the string is the control and also its accessible name. */
export const SKIP_AHEAD_LABEL = 'SKIP AHEAD';

/**
 * The word, never the colour. `condition_signal IS NULL` draws no dot at all, so there is nothing
 * to name — absence, not a name for an absence.
 */
export function conditionDotName(signal: ConditionSignal | null): string | null {
  if (signal === null) return null;
  return `Condition: ${signal}`;
}

/** §11.7: `Pin <project name>` / `Unpin <project name>`, on a real `<button>` with `aria-pressed`. */
export function pinControlName(projectName: string, isPinned: boolean): string {
  return `${isPinned ? 'Unpin' : 'Pin'} ${projectName}`;
}

/**
 * §11.7: an unnamed frame reads as a tier. This names the absence — and it is the accessibility
 * tree's half of "never render unknown as zero", so it may not contain a count.
 */
export const COMPLETION_NOT_COMPUTED_NAME = 'Completion not computed';

/** §11.7: a real `<button>` with `aria-expanded`, named by the header's summary line. */
export function eraChevronName(summaryLine: string): string {
  return summaryLine;
}

/**
 * Colour words no accessible name may contain. `silver`, `gold`, `brass` and `steel` are the
 * completion ladder's tier names; the rest are the paints a name would leak if someone described
 * a mark instead of its meaning.
 */
export const BANNED_NAME_WORDS: readonly string[] = [
  'blue',
  'red',
  'green',
  'amber',
  'orange',
  'yellow',
  'grey',
  'gray',
  'white',
  'black',
  'gold',
  'silver',
  'brass',
  'steel',
  'violet',
  'cyan',
];

const COLOUR_GUARD = new RegExp(`\\b(?:${BANNED_NAME_WORDS.join('|')})\\b`, 'i');

export function statesAColour(name: string): boolean {
  return COLOUR_GUARD.test(name);
}
