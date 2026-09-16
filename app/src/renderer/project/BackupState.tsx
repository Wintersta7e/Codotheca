/**
 * §25.3's backup-state line, on OVERVIEW between the description and the `NOTE` block.
 *
 * **The core holds the decision; this file holds the copy.** `ProjectDetail.backup` is a
 * three-variant enum and there is **no first-match table here** — one declaration, two consumers,
 * and the second one is §24's pre-flight in another process.
 *
 * The numbers and the age come from the primary `LocationDetail` the page already holds, so
 * neither side carries both the decision and the values.
 *
 * **Colour goes on the edge, never on the type.** `--pass`, `--warn` and `--fail` are §8.7
 * tokens; the sentence takes `--text-2`, §8.5.2's precedent — an unreachable or unbacked copy is
 * dimmed through its plate, never by dropping type below the decision floor.
 *
 * Per §19.4 phase 2 has no push, so this **states a fact and offers no control**.
 *
 * The word `clean` appears in none of these strings. Absence of dirty is *no changes as of T*,
 * and `scripts/check-destructive-tokens.mjs` would fail the build over it anyway.
 */
import type { CSSProperties, ReactElement } from 'react';

import type { BackupState, LocationDetail } from '../../generated/protocol';
import { formatAge } from '../derive/observation';

/**
 * `1 commit`, `2 commits`, `1 stash`, `2 stashes`. Both nouns in §25.3's sentence singularise,
 * and the plural is given rather than derived: a bare `+ 's'` renders `2 stashs`, which the
 * test caught on the first run.
 */
function count(n: number, one: string, many: string): string {
  return `${String(n)} ${n === 1 ? one : many}`;
}

/**
 * §25.3's sentence for `not_anywhere_else`, built from whichever halves are actually non-zero.
 *
 * A clause for a zero would print `0 stashes` beside a real commit count, which is the shape
 * §8.5.2 already refuses on the card.
 */
function onlyHereLine(location: LocationDetail): string {
  const parts: string[] = [];
  if (location.ahead !== null && location.ahead > 0) {
    parts.push(count(location.ahead, 'commit', 'commits'));
  }
  if (location.stashCount !== null && location.stashCount > 0) {
    parts.push(count(location.stashCount, 'stash', 'stashes'));
  }
  return `${parts.join(' and ')} exist only on this disk.`;
}

export interface BackupStateBlockProps {
  /** `null` is §25.3's fourth row: **no block at all**, never a fourth word. */
  readonly state: BackupState | null;
  /** The primary copy — the one §5.3 measures the project by. */
  readonly location: LocationDetail | null;
  readonly now: number;
}

export function BackupStateBlock({
  state,
  location,
  now,
}: BackupStateBlockProps): ReactElement | null {
  if (state === null || location === null) return null;

  const fetched = location.fetchHeadAt === null ? null : formatAge(now - location.fetchHeadAt);
  const parts: { readonly line: string; readonly plate: string; readonly edge: string } =
    state === 'only_copy'
      ? {
          line: 'No remote. This machine is the only copy.',
          plate: 'ONLY COPY ON EARTH',
          edge: '--fail',
        }
      : state === 'not_anywhere_else'
        ? {
            line: onlyHereLine(location),
            // The count is only as true as the fetch behind it, so the plate carries that age
            // where there is one and says nothing where there is not.
            plate: fetched === null ? 'NOT ANYWHERE ELSE' : `NOT ANYWHERE ELSE · AS OF ${fetched}`,
            edge: '--warn',
          }
        : {
            line: 'Nothing on this branch is only on this disk.',
            // The one row that claims currency, so the one row that names its observation.
            plate: fetched === null ? 'VERIFIED' : `VERIFIED ${fetched}`,
            edge: '--pass',
          };

  return (
    <section
      className="cp-backup"
      data-testid="cp-backup"
      data-state={state}
      style={{ '--cdt-backup-edge': `var(${parts.edge})` } as CSSProperties}
    >
      <span className="cp-backup-plate" data-testid="cp-backup-plate">
        {parts.plate}
      </span>
      <p className="cp-backup-line" data-testid="cp-backup-line">
        {parts.line}
      </p>
    </section>
  );
}
