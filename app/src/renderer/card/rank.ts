import type { ProjectRow } from '../../generated/protocol';
import type { TokenName } from '../theme/tokens';
import { isUncommitted, isUnpushed } from './chips';

/**
 * The rank, E to S: what shape a project is in right now. A composite the tier frame does not
 * already carry — the frame reads completion alone, the rank demotes it by decay and by open
 * flags. One owner for the grid card's band 3, the hero's band 3 and the list's `RANK` column.
 *
 * **Unknown never penalises and never ranks.** An uncomputed completion has no rank at all and
 * keeps `uncomputedRank`'s treatment; an unobserved condition, dirty state, push state or CI
 * check contributes nothing. Only an observed fact moves the index.
 */
export type Rank = 'E' | 'D' | 'C' | 'B' | 'A' | 'S';

/** Lowest first, so a rank's position is its index. */
const RANK_ORDER: readonly Rank[] = ['E', 'D', 'C', 'B', 'A', 'S'];

export type RankRow = Pick<
  ProjectRow,
  | 'completionLit'
  | 'completionApplicable'
  | 'conditionSignal'
  | 'isArchived'
  | 'isReference'
  | 'isDirty'
  | 'ahead'
  | 'ciGreen'
>;

/** The completion index before any adjustment: thresholds 1 / .85 / .7 / .5 / .3. */
export function completionIndex(lit: number, evaluable: number): number {
  const pct = lit / evaluable;
  if (pct >= 1) return 5;
  if (pct >= 0.85) return 4;
  if (pct >= 0.7) return 3;
  if (pct >= 0.5) return 2;
  if (pct >= 0.3) return 1;
  return 0;
}

/**
 * The condition band's adjustment. The design's bands map onto the shipped ladder by position:
 * its two worst (`neglected`, `abandoned`) keep their names and take −2, its `dormant` keeps its
 * name and takes −1, its freshest (`live`) takes +1. `idle` sits between live and dormant and
 * moves nothing. `offline` is a frozen band and `empty` was never alive, so neither is decay.
 *
 * Archived and reference rows take none: the design answers *archived* and *reference* before
 * any decay band, and §5.4a applies both at render time over the stored band, as the dot does.
 */
export function conditionAdjustment(
  row: Pick<RankRow, 'conditionSignal' | 'isArchived' | 'isReference'>,
): number {
  if (row.isArchived || row.isReference) return 0;
  switch (row.conditionSignal) {
    case 'live':
      return 1;
    case 'dormant':
      return -1;
    case 'neglected':
    case 'abandoned':
      return -2;
    case 'idle':
    case 'offline':
    case 'empty':
    case null:
      return 0;
  }
}

/**
 * `null` is *no rank*: completion is uncomputed, or nothing could be scored. A rank there would
 * be drawn from no measurement at all.
 */
export function rankOf(row: RankRow): Rank | null {
  const { completionLit: lit, completionApplicable: evaluable } = row;
  if (lit === null || evaluable === null || evaluable <= 0) return null;

  let index = completionIndex(lit, evaluable) + conditionAdjustment(row);
  // The same predicates as the `UNPUSHED` and `UNCOMMITTED` chips; an unobserved value is false.
  if (isUnpushed(row) || isUncommitted(row)) index -= 1;
  // The stored `ciGreen` check is the one CI fact. `pass`, `na`, `unknown` and null move nothing.
  if (row.ciGreen === 'fail') index -= 1;
  return RANK_ORDER[Math.min(RANK_ORDER.length - 1, Math.max(0, index))] ?? 'E';
}

export interface RankPaint {
  readonly rank: Rank;
  /** `E RANK` … `S RANK`, set under the glyph in the rank's own ink. */
  readonly label: `${Rank} RANK`;
  readonly inkToken: Extract<TokenName, `rank-${string}`>;
  /** The glyph's `text-shadow`; `null` draws none. Only S, A and B glow. */
  readonly glow: string | null;
  readonly accessibleName: `Rank ${Rank}`;
}

/**
 * The design's table. Low ranks read low through chroma and label, never luminance: C, D and E
 * are near-neutral and unlit, and every one of them is lighter than `--text-3`.
 */
const RANK_PAINT: Readonly<Record<Rank, Pick<RankPaint, 'inkToken' | 'glow'>>> = {
  S: { inkToken: 'rank-s', glow: '0 0 26px rgb(232 194 104 / .45)' },
  A: { inkToken: 'rank-a', glow: '0 0 24px rgb(178 138 236 / .40)' },
  B: { inkToken: 'rank-b', glow: '0 0 22px rgb(112 168 246 / .38)' },
  C: { inkToken: 'rank-c', glow: null },
  D: { inkToken: 'rank-d', glow: null },
  E: { inkToken: 'rank-e', glow: null },
};

export function rankPaint(row: RankRow): RankPaint | null {
  const rank = rankOf(row);
  if (rank === null) return null;
  return {
    rank,
    label: `${rank} RANK`,
    ...RANK_PAINT[rank],
    accessibleName: `Rank ${rank}`,
  };
}
