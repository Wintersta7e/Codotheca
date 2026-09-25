import type { ConditionSignal, ProjectRow } from '../../generated/protocol.js';

/**
 * Fields §8.3's projection needs that plan 02's `ProjectRow` does not yet carry. Each is
 * `null` = *not available*, never `false`: a `has:` attribute nothing computes must not read as
 * "this repository has no README".
 */
export interface ProjectRowExtras {
  readonly locationKind: 'win' | 'linux' | 'wsl' | null;
  readonly distro: string | null;
  readonly hasReadme: boolean | null;
  readonly hasLicense: boolean | null;
  readonly hasTests: boolean | null;
  readonly hasCi: boolean | null;
  // §23.7: `hasRemote` is **not** here any more. It is a field of `ProjectRow` itself, because
  // the shelf projection is `ProjectRow` and nothing else — a producer that is not a field on it
  // is not a producer. `has:remote` was answered by the core and dropped by the renderer, which
  // is R1/R35a/R40/R46 in projection form. `authoredByUser` followed it (R245): declared here and
  // carried by no row, it read null for every project and the classified figure was always 0.
  readonly hasSubmodules: boolean | null;
}

export type ShelfRow = ProjectRow & ProjectRowExtras;

const EXTRA_KEYS = [
  'locationKind',
  'distro',
  'hasReadme',
  'hasLicense',
  'hasTests',
  'hasCi',
  'hasSubmodules',
] as const;

export function toShelfRow(row: ProjectRow): ShelfRow {
  const carried = row as ProjectRow & Partial<ProjectRowExtras>;
  return {
    ...row,
    locationKind: carried.locationKind ?? null,
    distro: carried.distro ?? null,
    hasReadme: carried.hasReadme ?? null,
    hasLicense: carried.hasLicense ?? null,
    hasTests: carried.hasTests ?? null,
    hasCi: carried.hasCi ?? null,
    hasSubmodules: carried.hasSubmodules ?? null,
  };
}

/**
 * [p3] §35.3's membership rule and §35.2's ordering scalar, as one total function — the mirror of
 * `rank_of` in `core/src/projects/list.rs`, which `protocol/shelf/order-corpus.json` holds both
 * halves to.
 *
 * A number is *this row carries a reading, and its count is that number*; `null` is *tail*. The
 * case §30.1 forbids the writer to produce — a `live` reading with a null `scoredOpen` — resolves
 * to the tail rather than to a zero, because **a zero is a ranked value and never a tail value**.
 *
 * It re-applies none of §35.4's exclusions: §30's pipeline decides what the reading is, and this
 * reads it. `isReference`, `isArchived` and `lifecycle` are never consulted.
 *
 * **Here rather than in `page.ts` beside the comparator**: `projectionCapabilities` below needs
 * the same expression of *does this row carry a reading*, and a row module importing the page
 * module for it would point the dependency the wrong way down. One owner, read from both.
 */
export function rankOf(row: ShelfRow): number | null {
  switch (row.healthSummary.state) {
    case 'live':
    case 'frozen':
      return row.healthSummary.scoredOpen;
    case 'absent':
    case 'suppressed':
      return null;
  }
}

export interface ProjectionCapabilities {
  readonly authoredByUser: boolean;
  readonly location: boolean;
  readonly hasReadme: boolean;
  readonly hasLicense: boolean;
  readonly hasTests: boolean;
  readonly hasCi: boolean;
  readonly hasRemote: boolean;
  readonly hasSubmodules: boolean;
  /**
   * [p3] §35.5. **Read by the sort control and by no `has:` term** — it adds no grammar term and
   * `evaluate.ts`'s `answerable` switch is untouched. A row whose reading says `scoredOpen = 0`
   * answers it: it carries a reading. A row at `absent` or `suppressed` does not.
   */
  readonly health: boolean;
}

/** A capability is present when at least one row answers it. An empty projection answers nothing. */
export function projectionCapabilities(rows: readonly ShelfRow[]): ProjectionCapabilities {
  const answered = new Set<string>();
  for (const row of rows) {
    for (const key of EXTRA_KEYS) {
      if (row[key] !== null) answered.add(key);
    }
    if (row.authoredByUser !== null) answered.add('authoredByUser');
    // `hasRemote` is a wire field rather than an extra, so it is answered by any row at all —
    // and the same rule applies: an empty projection answers nothing.
    answered.add('hasRemote');
    // One expression of *does this row carry a reading*, shared with the comparator.
    if (rankOf(row) !== null) answered.add('health');
  }
  return {
    authoredByUser: answered.has('authoredByUser'),
    location: answered.has('locationKind'),
    hasReadme: answered.has('hasReadme'),
    hasLicense: answered.has('hasLicense'),
    hasTests: answered.has('hasTests'),
    hasCi: answered.has('hasCi'),
    hasRemote: answered.has('hasRemote'),
    hasSubmodules: answered.has('hasSubmodules'),
    health: answered.has('health'),
  };
}

export type ConditionOrNull = ConditionSignal | null;
