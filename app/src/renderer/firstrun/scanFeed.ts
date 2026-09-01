import { languageCode } from '../art/appearance';
import type { ProjectId } from '../../generated/protocol';

/**
 * §10.3a: the scan tile is deliberately not the shelf card, and this is the whole of what one
 * carries. Recorded here because the divergence gets "corrected" back otherwise and the 10 px
 * name is lost to the scrim.
 */
export interface ScanTile {
  readonly id: ProjectId;
  readonly name: string;
  /** `null` is *not yet classified*, which the stripe draws as `--absent`, never as absent. */
  readonly primaryLanguage: string | null;
}

export interface TallyChip {
  readonly sigil: string;
  readonly count: number;
}

export interface ScanFeedState {
  /** §10.3a's 64 px headline: the project count, never directories and never repositories. */
  readonly found: number;
  readonly tiles: readonly ScanTile[];
  readonly pending: readonly ScanTile[];
  /** Absorbed tiles, fading for 200 ms; the next flush removes them. */
  readonly merging: ReadonlySet<ProjectId>;
  readonly tally: readonly TallyChip[];
  readonly walkedDirs: number;
  readonly foundRepos: number;
  readonly finished: boolean;
  /**
   * Merges observed since the last progress frame. The core's `indexedProjects` already
   * reflects a merge by the following frame, so without this the count either lags a fade or
   * subtracts the same merge twice.
   */
  readonly mergesSinceProgress: number;
}

export type ScanFeedEvent =
  | {
      readonly kind: 'upserted';
      readonly id: ProjectId;
      readonly name: string;
      readonly primaryLanguage: string | null;
    }
  | { readonly kind: 'merged'; readonly from: ProjectId; readonly into: ProjectId }
  | {
      readonly kind: 'progress';
      readonly indexedProjects: number;
      readonly walkedDirs: number;
      readonly foundRepos: number;
    }
  | { readonly kind: 'flush' }
  | { readonly kind: 'finished' };

export const INITIAL_SCAN_FEED: ScanFeedState = {
  found: 0,
  tiles: [],
  pending: [],
  merging: new Set(),
  tally: [],
  walkedDirs: 0,
  foundRepos: 0,
  finished: false,
  mergesSinceProgress: 0,
};

/** §10.3a: at most 7 chips, sorted by count descending. */
export const TALLY_CAP = 7;

/** §10.2 and §10.3a. Each is a preview computed over what is indexed at that instant. */
export const MILESTONES = [10, 50, 100, 250, 500] as const;

function tallyOf(tiles: readonly ScanTile[], pending: readonly ScanTile[]): readonly TallyChip[] {
  const counts = new Map<string, number>();
  for (const tile of [...tiles, ...pending]) {
    // A project with no classified language contributes to no chip. An `UNKNOWN` chip would be
    // "unknown as zero" wearing a label.
    if (tile.primaryLanguage === null) continue;
    const sigil = languageCode(tile.primaryLanguage);
    counts.set(sigil, (counts.get(sigil) ?? 0) + 1);
  }
  return [...counts]
    .map(([sigil, count]) => ({ sigil, count }))
    .sort((a, b) => b.count - a.count || a.sigil.localeCompare(b.sigil))
    .slice(0, TALLY_CAP);
}

function upsertInto(
  list: readonly ScanTile[],
  tile: ScanTile,
): { readonly list: readonly ScanTile[]; readonly replaced: boolean } {
  const at = list.findIndex((t) => t.id === tile.id);
  if (at === -1) return { list, replaced: false };
  const next = [...list];
  next[at] = tile;
  return { list: next, replaced: true };
}

export function scanFeedReducer(state: ScanFeedState, event: ScanFeedEvent): ScanFeedState {
  switch (event.kind) {
    case 'upserted': {
      const tile: ScanTile = {
        id: event.id,
        name: event.name,
        primaryLanguage: event.primaryLanguage,
      };
      // §10.3: the second wave powers a tile on where it already stands. Re-appending would
      // re-sort, which §10.3 bans.
      const inTiles = upsertInto(state.tiles, tile);
      if (inTiles.replaced) {
        return { ...state, tiles: inTiles.list, tally: tallyOf(inTiles.list, state.pending) };
      }
      const inPending = upsertInto(state.pending, tile);
      const pending = inPending.replaced ? inPending.list : [...state.pending, tile];
      return { ...state, pending, tally: tallyOf(state.tiles, pending) };
    }
    case 'merged': {
      // §10.3a: the absorbed tile fades over 200 ms and, in the same frame, the count drops.
      const known =
        state.tiles.some((t) => t.id === event.from) ||
        state.pending.some((t) => t.id === event.from);
      if (!known || state.merging.has(event.from)) return state;
      const merging = new Set(state.merging);
      merging.add(event.from);
      return {
        ...state,
        merging,
        found: Math.max(0, state.found - 1),
        mergesSinceProgress: state.mergesSinceProgress + 1,
      };
    }
    case 'progress':
      // The core's count already reflects every merge it has committed, so the local offset is
      // discharged rather than applied again.
      return {
        ...state,
        found: event.indexedProjects,
        walkedDirs: event.walkedDirs,
        foundRepos: event.foundRepos,
        mergesSinceProgress: 0,
      };
    case 'flush': {
      const tiles = [...state.tiles, ...state.pending].filter((t) => !state.merging.has(t.id));
      return { ...state, tiles, pending: [], merging: new Set(), tally: tallyOf(tiles, []) };
    }
    case 'finished':
      return { ...state, finished: true };
  }
}

/**
 * §10.2: milestones at 10/50/100/250/500. The highest crossed by one batch wins — each is a
 * preview of what is indexed at that instant, so a queue of two would render one stale figure.
 * Crossing downwards, which a merge can do, fires nothing.
 */
export function milestoneCrossed(before: number, after: number): number | null {
  if (after <= before) return null;
  let hit: number | null = null;
  for (const m of MILESTONES) {
    if (before < m && after >= m) hit = m;
  }
  return hit;
}

/** §10.3a's scan line. These two counts never appear in the 64 px headline. */
export function scanLineText(walkedDirs: number, foundRepos: number): string {
  return `${walkedDirs.toLocaleString('en-US')} directories walked · ${foundRepos.toLocaleString('en-US')} repositories`;
}
