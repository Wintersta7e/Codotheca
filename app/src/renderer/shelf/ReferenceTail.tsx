import type { CSSProperties, ReactElement } from 'react';
import type { ProjectId } from '../../generated/protocol.js';
import { conditionDotName } from '../a11y/names.js';
import { conditionDot } from '../derive/condition.js';
import { formatTrackedBytes } from '../format/size.js';
import type { ShelfRow } from './row.js';

/**
 * §8.1: the design's "excluded from decay, health, quests and stats" names three things phase 1
 * does not have. This is what is left that is true.
 */
export function referenceSummary(count: number): string {
  return `${String(count)} repositories with no commits by you · excluded from your stats`;
}

export interface ReferenceTailProps {
  readonly rows: readonly ShelfRow[];
  readonly onOpen: (id: ProjectId) => void;
}

export function ReferenceTail({ rows, onOpen }: ReferenceTailProps): ReactElement | null {
  if (rows.length === 0) return null;
  return (
    <section className="cdt-reference-tail" aria-label={referenceSummary(rows.length)}>
      <header className="cdt-reference-head">
        <span>REFERENCE</span>
        <span className="cdt-reference-summary">{referenceSummary(rows.length)}</span>
      </header>
      {/* Uncapped by rule (§8.1): `refs.slice(0, 40)` is a fixture convenience, and forty is
          exactly the wrong number to silently stop at. These are text rows, not cards; the grid
          above is what the virtualizer exists for. */}
      {rows.map((row) => {
        const dot = conditionDot({
          signal: row.conditionSignal,
          isReference: row.isReference,
          isArchived: row.isArchived,
        });
        // §5.4a gives every reference row the same unfilled ring before it reads the band, so the
        // mark means "reference" and has no name of its own. Naming it would name a colour; the
        // block is already called REFERENCE, so it is hidden rather than announced.
        const dotName = conditionDotName(row.conditionSignal);
        return (
          <button
            type="button"
            key={row.id}
            className="cdt-reference-row"
            onClick={() => {
              onOpen(row.id);
            }}
          >
            {dot === null ? null : (
              <span
                className="cdt-condition-dot"
                aria-label={dotName ?? undefined}
                aria-hidden={dotName === null ? true : undefined}
                style={
                  {
                    '--cdt-dot-fill': dot.fill ?? 'transparent',
                    '--cdt-dot-ring': dot.ring ?? 'none',
                  } as CSSProperties
                }
              />
            )}
            <span className="cdt-reference-name">{row.name}</span>
            <span className="cdt-reference-lang">{row.primaryLanguage ?? ''}</span>
            {/* An unmeasured inventory renders the uncomputed mark, never a zero — a measured
                `0 MB` is a fact and prints as one. */}
            <span className="cdt-reference-size">
              {row.sizeTrackedBytes === null ? '—' : formatTrackedBytes(row.sizeTrackedBytes)}
            </span>
          </button>
        );
      })}
    </section>
  );
}
