import type { LocationId, ProjectRow } from '../../generated/protocol.js';

/** §8.6: the rendered list is capped at 40 rows; the count states the full match set. */
export const PALETTE_ROW_CAP = 40;

/** §11.2's precedent for a surface that cannot act: state the reason, never a dead control. */
export const PALETTE_UNAVAILABLE_TEXT = 'NO COPY ON THIS MACHINE';

/** §8.6: `is_reference = 0 AND is_hidden = 0 AND submodule_path IS NULL`. */
export function isPaletteCandidate(row: ProjectRow): boolean {
  return !row.isReference && !row.isHidden && !row.isSubmodule;
}

/** §8.6: matching on name and `primary_language`. Not description, not owner, not path. */
export function paletteMatches(row: ProjectRow, needle: string): boolean {
  const n = needle.trim().toLowerCase();
  if (n === '') return true;
  if (row.name.toLowerCase().includes(n)) return true;
  const lang = row.primaryLanguage;
  return lang !== null && lang.toLowerCase().includes(n);
}

export interface PaletteSelection {
  /** At most `PALETTE_ROW_CAP`, in render order. */
  readonly rows: readonly ProjectRow[];
  /** Every match, uncapped. */
  readonly matched: number;
  /** Every candidate under the predicate — the denominator, never the library size. */
  readonly total: number;
}

export function selectPaletteRows(all: readonly ProjectRow[], query: string): PaletteSelection {
  const candidates = all.filter(isPaletteCandidate);
  const hits = candidates.filter((row) => paletteMatches(row, query));
  const ordered = [...hits].sort((a, b) => b.lastTouchedAt - a.lastTouchedAt || a.id - b.id);
  return {
    rows: ordered.slice(0, PALETTE_ROW_CAP),
    matched: hits.length,
    total: candidates.length,
  };
}

/** §8.6's query bar: `<matched> OF <total>`. */
export function paletteCountText(selection: PaletteSelection): string {
  return `${String(selection.matched)} OF ${String(selection.total)}`;
}

export type PaletteRowAction =
  { readonly kind: 'launch'; readonly locationId: LocationId } | { readonly kind: 'unavailable' };

export function paletteRowAction(row: ProjectRow): PaletteRowAction {
  const location = row.primaryLocation;
  if (location === null) return { kind: 'unavailable' };
  return { kind: 'launch', locationId: location.id };
}
