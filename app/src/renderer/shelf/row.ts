import type { ConditionSignal, ProjectRow } from '../../generated/protocol.js';

/**
 * Fields §8.3's projection needs that plan 02's `ProjectRow` does not yet carry. Each is
 * `null` = *not available*, never `false`: a `has:` attribute nothing computes must not read as
 * "this repository has no README".
 */
export interface ProjectRowExtras {
  readonly authoredByUser: boolean | null;
  readonly locationKind: 'win' | 'linux' | 'wsl' | null;
  readonly distro: string | null;
  readonly hasReadme: boolean | null;
  readonly hasLicense: boolean | null;
  readonly hasTests: boolean | null;
  readonly hasCi: boolean | null;
  readonly hasRemote: boolean | null;
  readonly hasSubmodules: boolean | null;
}

export type ShelfRow = ProjectRow & ProjectRowExtras;

const EXTRA_KEYS = [
  'authoredByUser',
  'locationKind',
  'distro',
  'hasReadme',
  'hasLicense',
  'hasTests',
  'hasCi',
  'hasRemote',
  'hasSubmodules',
] as const;

export function toShelfRow(row: ProjectRow): ShelfRow {
  const carried = row as ProjectRow & Partial<ProjectRowExtras>;
  return {
    ...row,
    authoredByUser: carried.authoredByUser ?? null,
    locationKind: carried.locationKind ?? null,
    distro: carried.distro ?? null,
    hasReadme: carried.hasReadme ?? null,
    hasLicense: carried.hasLicense ?? null,
    hasTests: carried.hasTests ?? null,
    hasCi: carried.hasCi ?? null,
    hasRemote: carried.hasRemote ?? null,
    hasSubmodules: carried.hasSubmodules ?? null,
  };
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
}

/** A capability is present when at least one row answers it. An empty projection answers nothing. */
export function projectionCapabilities(rows: readonly ShelfRow[]): ProjectionCapabilities {
  const answered = new Set<string>();
  for (const row of rows) {
    for (const key of EXTRA_KEYS) {
      if (row[key] !== null) answered.add(key);
    }
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
  };
}

export type ConditionOrNull = ConditionSignal | null;
