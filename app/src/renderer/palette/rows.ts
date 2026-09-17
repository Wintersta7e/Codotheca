import type { LocationId, ProjectId, ProjectRow } from '../../generated/protocol.js';

/** §8.6: the rendered list is capped at 40 rows; the count states the full match set. */
export const PALETTE_ROW_CAP = 40;

/** §11.2's precedent for a surface that cannot act: state the reason, never a dead control. */
export const PALETTE_UNAVAILABLE_TEXT = 'NO COPY ON THIS MACHINE';

/**
 * §24.5's trailing slot on a not-cloned row. **The palette does not clone**: the ellipsis is the
 * promise that the control opens the place where the destination is chosen. Binding a launcher's
 * one keystroke to a multi-minute network write into a destination invisible from a 46 px query
 * bar is the wrong trade — the same family as *roasting only inside an opened project card*.
 */
export const PALETTE_INSTALL_TEXT = '↵ INSTALL…';

/**
 * What `↵` asks the project page to focus as it opens it, and the only value there is.
 *
 * The consumer is p2-24 Task 19's Install control, which does not exist yet: the argument is
 * accepted and ignored by today's `openProject` (`app/src/renderer/App.tsx:98`). Nothing here
 * asserts the affordance received focus, because nothing here can see it.
 */
export type PaletteOpenFocus = 'install';

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

/**
 * §24.5's leading sort key, read through `paletteRowAction` so the order and the trailing slot
 * cannot disagree: the rows that sort last are exactly the rows that offer INSTALL.
 *
 * It is the tail-section rule expressed in the one dimension a flat list has, and it is what
 * keeps blueprints from consuming `PALETTE_ROW_CAP`. Whatever §23 gives a zero-location project
 * for `last_touched_at` must not defeat it, so it sits ahead of both existing keys.
 */
function sortsAfterEveryLocatedRow(row: ProjectRow): number {
  return paletteRowAction(row).kind === 'install' ? 1 : 0;
}

export function selectPaletteRows(all: readonly ProjectRow[], query: string): PaletteSelection {
  const candidates = all.filter(isPaletteCandidate);
  const hits = candidates.filter((row) => paletteMatches(row, query));
  const ordered = [...hits].sort(
    (a, b) =>
      sortsAfterEveryLocatedRow(a) - sortsAfterEveryLocatedRow(b) ||
      b.lastTouchedAt - a.lastTouchedAt ||
      a.id - b.id,
  );
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
  | { readonly kind: 'launch'; readonly locationId: LocationId }
  | { readonly kind: 'install'; readonly projectId: ProjectId }
  | { readonly kind: 'unavailable' };

/**
 * Three facts, never two.
 *
 * `install` is §23.1's one predicate — `primaryLocation === null`, the same expression
 * `subLineInputFor` reads and never a second one. On the wire `presence` is
 * `primary.map(|l| l.presence)` (`core/src/projects/rows.rs:368`) over a `pick_primary` that does
 * **not** filter on presence (`:112`), so `presence === null` is that same predicate rendered,
 * not an independent one.
 *
 * `unavailable` is therefore the *other* fact §24.5 keeps its own words for: copies exist and
 * none of them can be opened. `unscanned` is not one of those — nobody has looked at it, and
 * claiming NO COPY ON THIS MACHINE about it would be the false-absence defect this arm exists
 * to prevent.
 */
export function paletteRowAction(row: ProjectRow): PaletteRowAction {
  const location = row.primaryLocation;
  if (location === null) return { kind: 'install', projectId: row.id };
  if (row.presence === 'offline' || row.presence === 'missing') return { kind: 'unavailable' };
  return { kind: 'launch', locationId: location.id };
}
