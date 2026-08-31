import { statusChips } from '../card/chips.js';
import type { ChipRow, StatusChip } from '../card/chips.js';

/**
 * §8.0c column 8. The derivation, the order, the cap and the accessible names are the card
 * module's; what is different here is only the geometry and the direction the column overflows in.
 */
export const LIST_CHIP_TYPE = {
  fontPx: 7,
  weight: 700,
  tracking: '.1em',
  padding: '2px 4px',
} as const;

export const LIST_CHIP_COLUMN_PX = 128;

/**
 * Right-aligned in a fixed 128px column, so DOM order puts the highest-priority chip last and any
 * overflow clips from the low-priority end — §8.1's truncation order applied to a row: what is
 * dropped is the least load-bearing element, never the first one.
 */
export function listRowChips(
  row: ChipRow,
  now: number,
  firstRunCompletedAt: number | null,
): readonly StatusChip[] {
  return [...statusChips(row, now, firstRunCompletedAt)].reverse();
}

/** Clipping is visual only; the row's accessible name still carries every chip. */
export function listChipsLabel(chips: readonly StatusChip[]): string {
  return chips.map((chip) => chip.accessibleName).join(', ');
}
