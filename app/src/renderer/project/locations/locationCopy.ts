/**
 * §8.5.2's copy. Every string this panel can render lives here, so the panel is arrangement and
 * the sentences are testable without a DOM.
 *
 * The panel answers "which copy is current", so every string in it is decision-carrying and
 * §8.7's floor applies across all of it: `--text-4` and `--text-5` appear in its stylesheet
 * nowhere. An unreachable copy is dimmed through its spine and its tag plate, never by dropping
 * type below the floor.
 */
import type { AssociationKind, LocationDetail, LocationKind } from '../../../generated/protocol';
import { formatAge } from '../../derive/observation';
import { formatTrackedBytes } from '../../format/size';

export type LocationStateWord =
  | 'SAME COMMIT'
  | 'DIFFERENT COMMIT'
  | 'NOT COMPARED'
  | 'OFFLINE'
  | 'MISSING'
  | 'NOT SCANNED'
  /** [p2] §24.6a. Checked **before** `presence`, which still reads `present` until a scan runs. */
  | 'UNINSTALLED';

export const OFFLINE_NOTE =
  'The drive is not mounted. This copy is frozen, not rotting — its condition stops here rather than decaying.';

export const MISSING_NOTE =
  'The drive is mounted and the folder is not on it. It may have been moved or deleted; Codotheca removes nothing either way.';

/**
 * [p2] §24.6a's note, in the panel's voice.
 *
 * It says the tile is kept and the copy is re-clonable, and it carries none of `clean`, `delete`
 * or `remove` — the three words §24.2c bans from a rendered string.
 */
export const UNINSTALLED_NOTE =
  'You uninstalled this copy. The project stays on your shelf and its history stays on the remote; installing it again brings the same work back.';

export const DIFFERENT_COMMIT_NOTE =
  'This copy is on a different commit. Opening it is not the same as opening the project.';

/**
 * §1.3, §8.5.2: a present copy is compared by identity, never by direction. A directional word
 * needs a merge-base across two object stores, which needs a fetch, which §17 forbids.
 */
export function stateWord(location: LocationDetail): LocationStateWord {
  // [p2] §24.6a: `removedAt` wins, and it is tested **first**. `presence` is a scan observation
  // and the scan has not run since the removal, so it still reads `present` for a directory that
  // is gone — testing it first would render `SAME COMMIT` about nothing.
  if (location.removedAt !== null) return 'UNINSTALLED';
  switch (location.presence) {
    case 'offline':
      return 'OFFLINE';
    case 'missing':
      return 'MISSING';
    case 'unscanned':
      return 'NOT SCANNED';
    case 'present':
      break;
  }
  switch (location.headComparison) {
    case 'same_commit':
      return 'SAME COMMIT';
    case 'different_commit':
      return 'DIFFERENT COMMIT';
    case 'not_compared':
      return 'NOT COMPARED';
  }
}

export interface LocationFact {
  readonly key: string;
  readonly value: string;
}

/**
 * §8.5.2, §6: a count is only as true as the fetch that produced it, so it carries that age.
 *
 * [p2] Exported for §25.1's `BEHIND` block, which renders the same stored value at a different
 * size. It is already the single owner of `no fetch recorded` and `last fetch <age>`, and a
 * second copy on that tab would be R12 exactly.
 */
export function fetchClause(fetchHeadAt: number | null, now: number): string {
  return fetchHeadAt === null ? 'no fetch recorded' : `last fetch ${formatAge(now - fetchHeadAt)}`;
}

export function locationFacts(args: {
  location: LocationDetail;
  sizeTrackedBytes: number | null;
  now: number;
}): LocationFact[] {
  const { location, sizeTrackedBytes, now } = args;
  const out: LocationFact[] = [];

  if (location.presence === 'offline' || location.presence === 'missing') {
    if (location.lastSeenAt !== null) {
      out.push({ key: 'LAST SEEN', value: formatAge(now - location.lastSeenAt) });
    }
    // Criterion 63: the branch as last observed, and no volume or drive name at all.
    if (location.presence === 'offline' && location.branch !== null) {
      out.push({ key: 'BRANCH', value: location.branch });
    }
    return out;
  }

  if (location.presence === 'unscanned') {
    // The root's own path needs a command the schema does not have; see this plan's gap 2.
    if (location.coveringRootId !== null) out.push({ key: 'ROOT', value: 'DISABLED' });
    return out;
  }

  if (location.branch !== null) out.push({ key: 'BRANCH', value: location.branch });

  if (location.ahead === null && location.behind === null) {
    out.push({ key: 'UPSTREAM', value: 'no upstream' });
  } else {
    const clause = fetchClause(location.fetchHeadAt, now);
    // A measured zero is level: there is nothing to report, and `AHEAD 0` is the shape
    // criterion 63 bans on the card for exactly this reason.
    if (location.ahead !== null && location.ahead > 0) {
      out.push({ key: 'AHEAD', value: `${String(location.ahead)} · ${clause}` });
    }
    if (location.behind !== null && location.behind > 0) {
      out.push({ key: 'BEHIND', value: `${String(location.behind)} · ${clause}` });
    }
  }

  // §1.2, §5.3: both size columns live on `project`, measured at the primary. So it renders once.
  if (location.isPrimary && sizeTrackedBytes !== null) {
    out.push({ key: 'TRACKED', value: formatTrackedBytes(sizeTrackedBytes) });
  }
  return out;
}

export type LocationActionId = 'open' | 'reveal' | 'relocate' | 'enableRoot' | 'install';

export function locationActions(location: LocationDetail): LocationActionId[] {
  // [p2] §24.6a: an uninstalled copy offers INSTALL and **never RELOCATE**. `missing` means the
  // scan looked and did not find it, so relocating asks *where did it go?*; there is nowhere to
  // relocate a copy the user deliberately removed.
  if (location.removedAt !== null) return ['install'];
  switch (location.presence) {
    case 'present':
      return ['open', 'reveal'];
    case 'offline':
    case 'missing':
      return ['relocate'];
    case 'unscanned':
      return location.coveringRootId === null ? [] : ['enableRoot'];
  }
}

/** §17: no phase-1 action mutates disk. `RELOCATE` rewrites one row's path and nothing else. */
export function actionLabel(id: LocationActionId): string {
  switch (id) {
    case 'open':
      return 'OPEN';
    case 'reveal':
      return 'REVEAL';
    case 'relocate':
      return 'RELOCATE';
    case 'enableRoot':
      return 'ENABLE ROOT';
    case 'install':
      return 'INSTALL';
  }
}

/**
 * §4bis.4 makes the `\\wsl.localhost\…` form display-only, never used to scan or launch, so a
 * WSL copy drawn as a bare path invites exactly the wrong reading.
 */
export function kindChip(kind: LocationKind, distro: string): string {
  if (kind === 'win') return 'WIN';
  if (kind === 'linux') return 'LINUX';
  return distro === '' ? 'WSL' : `WSL · ${distro}`;
}

export function rowTag(isPrimary: boolean): 'PRIMARY' | 'COPY' {
  return isPrimary ? 'PRIMARY' : 'COPY';
}

const EXCEPTIONS = [
  { presence: 'offline', word: 'OFFLINE' },
  { presence: 'missing', word: 'MISSING' },
  { presence: 'unscanned', word: 'NOT SCANNED' },
] as const;

/** `ALL REACHABLE` is a claim, so it is made only when every row is `present`. */
export function headerNote(locations: readonly LocationDetail[]): string {
  if (locations.length <= 1) return 'ONE COPY ON THIS MACHINE';
  const head = `${String(locations.length)} COPIES`;
  const named = EXCEPTIONS.flatMap(({ presence, word }) => {
    // [p2] An uninstalled copy is counted by its own clause below, never by its stale `presence`.
    const n = locations.filter((l) => l.removedAt === null && l.presence === presence).length;
    return n === 0 ? [] : [`${n === 1 ? 'ONE' : String(n)} ${word}`];
  });
  // [p2] §24.6a: `ALL REACHABLE` is a **claim**, and it must not be made while a copy is
  // uninstalled. The note names it instead.
  const uninstalled = locations.filter((l) => l.removedAt !== null).length;
  const clauses =
    uninstalled === 0
      ? named
      : [...named, `${uninstalled === 1 ? 'ONE' : String(uninstalled)} UNINSTALLED`];
  return clauses.length === 0 ? `${head} · ALL REACHABLE` : [head, ...clauses].join(' · ');
}

/**
 * The superseded footer asserted the exact inference §1.1 forbids — a fork and its upstream share
 * a root commit by definition. This is the evidence that actually made the association.
 */
export function footerText(kind: AssociationKind | null, locationCount: number): string | null {
  if (locationCount < 2 || kind === null) return null;
  switch (kind) {
    case 'definitive':
      return 'SAME GIT DIRECTORY · LINKED WORKTREES';
    case 'strong':
      return 'SAME LINEAGE · SAME REMOTE';
    case 'inferred':
      return 'SAME LINEAGE · ONE COPY HAS NO REMOTE · INFERRED';
    case 'manual':
      return 'MERGED BY YOU';
  }
}

export function locationNote(location: LocationDetail): string | null {
  // First, for the same reason `stateWord` tests it first.
  if (location.removedAt !== null) return UNINSTALLED_NOTE;
  if (location.presence === 'offline') return OFFLINE_NOTE;
  if (location.presence === 'missing') return MISSING_NOTE;
  if (location.presence === 'present' && location.headComparison === 'different_commit') {
    return DIFFERENT_COMMIT_NOTE;
  }
  return null;
}
