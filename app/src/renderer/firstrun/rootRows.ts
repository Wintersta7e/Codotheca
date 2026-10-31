import type { RootAdd, RootSuggestion } from '../../generated/protocol';
import { REFUSAL_REASONS, UNCOMPUTED_GLYPH, UNKNOWN_GLYPH } from './copy';

export interface RootRow {
  /**
   * The row's identity, built from display parts only. §2.5: the renderer never constructs,
   * compares or returns a path.
   */
  readonly key: string;
  readonly pathDisplay: string;
  /** The WSL distro the row's path lives in; empty for a native path. */
  readonly distro: string;
  /** True when a named source — git config or an editor's recent list — proposed the row. */
  readonly sourced: boolean;
  readonly provenance: string;
  readonly count: string;
  readonly countUnit: 'HITS' | 'PROJECTS';
  readonly ticked: boolean;
  /** False for the three absolute refusals, which are listed with no tick slot at all. */
  readonly tickable: boolean;
}

/**
 * A suggestion's identity as the screen can see it: `kind`, `distro` and `pathDisplay`.
 *
 * The display string alone is not one. Every WSL distro row renders the same home-parent path,
 * so a key of the path gave two distros one tick between them.
 */
export function suggestionKey(
  row: Pick<RootSuggestion, 'kind' | 'distro' | 'pathDisplay'>,
): string {
  return JSON.stringify([row.kind, row.distro, row.pathDisplay]);
}

/**
 * The row's second line. §10.1b: this is the trust argument, so it is never blank and never
 * approximate — a row with no stated source has no reason to be ticked.
 */
export function provenanceLabel(row: RootSuggestion): string {
  switch (row.provenance) {
    case 'gitconfig':
      return `GITCONFIG · includeIf gitdir:${row.pathDisplay}/`;
    case 'editor_recent':
      return row.provenanceDetail === 'jetbrains'
        ? 'JETBRAINS · RECENT PROJECTS'
        : 'VS CODE · RECENT WORKSPACES';
    case 'convention':
      return 'A COMMON PLACE FOR REPOSITORIES · NO SOURCE NAMED IT';
    case 'cloud_synced':
      return 'CLOUD-SYNCED · SCANNING MAY TRIGGER DOWNLOADS';
    case 'system_root':
      return 'SYSTEM ROOT';
    case 'distro':
      // §13: the tick *is* the consent to start a stopped distro.
      return row.provenanceDetail === 'running'
        ? 'A DISTRO ON THIS MACHINE'
        : 'NOT RUNNING · SCANNING WILL START IT';
    case 'dialog':
      return 'CHOSEN BY YOU';
    default:
      return 'NO SOURCE NAMED IT';
  }
}

/**
 * §10.1b: the count is provenance hits, not repositories, until a walk has run.
 *
 * Nothing has been walked when this screen is drawn, and the standfirst promises exactly that,
 * so a repository count here would be fabricated. A negative `projectCount` is the sentinel the
 * settings panel uses for an unscanned or offline root (§4.6) and reads `?`. Never `0`.
 */
export function countGlyph(
  hits: number | null,
  projectCount: number | null,
): { readonly text: string; readonly unit: 'HITS' | 'PROJECTS' } {
  if (projectCount !== null) {
    return { text: projectCount < 0 ? UNCOMPUTED_GLYPH : String(projectCount), unit: 'PROJECTS' };
  }
  return { text: hits === null ? UNKNOWN_GLYPH : String(hits), unit: 'HITS' };
}

export function toRow(row: RootSuggestion, ticked: boolean): RootRow {
  const count = countGlyph(row.hits, null);
  return {
    key: suggestionKey(row),
    pathDisplay: row.pathDisplay,
    distro: row.distro,
    sourced: row.provenance === 'gitconfig' || row.provenance === 'editor_recent',
    provenance: provenanceLabel(row),
    count: count.text,
    countUnit: count.unit,
    ticked,
    tickable: true,
  };
}

/** §10.1b: refusals are drawn, not hidden — and they are not one kind. */
export function refusedRow(add: RootAdd, pathDisplay: string): RootRow {
  const refusal = add.refusedBecause;
  const confirmable = refusal === 'too_many_directories';
  return {
    key: pathDisplay,
    pathDisplay,
    distro: '',
    sourced: false,
    provenance: refusal === null ? 'CHOSEN BY YOU' : REFUSAL_REASONS[refusal],
    count: UNKNOWN_GLYPH,
    countUnit: 'HITS',
    ticked: confirmable,
    tickable: confirmable,
  };
}

export function initialTicks(rows: readonly RootSuggestion[]): ReadonlySet<string> {
  return new Set(rows.filter((r) => r.preTicked).map(suggestionKey));
}
