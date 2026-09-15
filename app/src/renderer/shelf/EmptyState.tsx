import type { ReactElement } from 'react';
import type { QueryAst } from '../../shared/query/ast.js';
import { effectiveQueryText, ignoredClause } from '../../shared/query/format.js';
import { formatClock } from '../derive/observation.js';
import type { ShelfCounts } from './counts.js';

export type EmptyStateVariant = 'noMatch' | 'noMatchWithIgnored' | 'noLibrary' | 'uncomputed';

/**
 * What the projection said, in the three states it actually has.
 *
 * `'uncomputed'` is *the core has not answered* — a read still in flight, or one that failed and
 * left `useLibrary` holding `null`. It is **not** `'empty'`, and the distinction is the whole
 * point: a shelf that has read nothing may not say *"no repositories under the enabled roots"*,
 * which is a claim about the user's disk made out of the app's own silence.
 */
export type LibraryPresence = 'uncomputed' | 'empty' | 'present';

export const CLEAR_QUERY_LABEL = 'CLEAR THE QUERY' as const;
export const ADD_ROOT_LABEL = 'ADD A SCAN ROOT' as const;
/** §8.0 grants the empty-library variant "a link into the scan summary when the last run
 *  recorded problems" and gives no wording. §11.1 owns what the summary says; this is only the
 *  way in, phrased on §8.0's own noun. */
export const SCAN_PROBLEMS_LABEL = 'SEE THE PROBLEMS' as const;

const NO_MATCH_HEADING = 'NOTHING MATCHES THAT';
const NO_LIBRARY_HEADING = 'NOTHING INDEXED YET';
/** §7.7a's pair, one surface up: the label says which figure is missing, the glyph stands in
 *  for it. Neither says anything about what is on the user's disk. */
const UNCOMPUTED_HEADING = 'NOT COMPUTED';
const UNCOMPUTED_GLYPH = '—';

export interface EmptyStateModel {
  readonly variant: EmptyStateVariant;
  readonly heading: string;
  readonly reason: string;
  /** `null` offers no button: an unread library has nothing for one to act on. */
  readonly action: string | null;
}

export interface EmptyStateInput {
  readonly ast: QueryAst;
  readonly counts: ShelfCounts;
  readonly library: LibraryPresence;
  /** Unix seconds. The empty-library line states when it looked; it never implies now. */
  readonly now: number;
}

/**
 * Three variants where both source documents describe one.
 *
 * The reason line is built from the **effective** query — the terms that ran — because §8.3
 * drops a soft-errored term before the filter executes, so echoing the raw input names a filter
 * that was never applied. The empty-library line carries no count at all: the prototype's
 * `<n> projects in the library, none of them shown` asserts at `n = 0` that projects exist and
 * are being hidden.
 */
export function emptyStateModel(input: EmptyStateInput): EmptyStateModel {
  // An unread library is uncomputed, and §7.7a's pair — the label and the em dash — is how this
  // app already says so. A packaged build printed the absence sentence below over twenty-six
  // indexed projects, because a read that had not happened and a read that found nothing were
  // one boolean by the time they reached this function.
  if (input.library === 'uncomputed') {
    return {
      variant: 'uncomputed',
      heading: UNCOMPUTED_HEADING,
      reason: UNCOMPUTED_GLYPH,
      action: null,
    };
  }
  if (input.library === 'empty') {
    return {
      variant: 'noLibrary',
      heading: NO_LIBRARY_HEADING,
      reason: `No repositories under the enabled roots as of ${formatClock(input.now)}.`,
      action: ADD_ROOT_LABEL,
    };
  }
  const ignored = ignoredClause(input.ast.ignored);
  return {
    variant: ignored.length === 0 ? 'noMatch' : 'noMatchWithIgnored',
    heading: NO_MATCH_HEADING,
    reason:
      `Query: ${effectiveQueryText(input.ast)}` +
      ` · ${String(input.counts.total)} projects searched` +
      ` · ${String(input.counts.reference)} reference rows excluded${ignored}`,
    action: CLEAR_QUERY_LABEL,
  };
}

export interface EmptyStateProps {
  readonly model: EmptyStateModel;
  readonly onClearQuery: () => void;
  readonly onAddScanRoot: () => void;
  /** Non-null when the last run recorded problems. Only the empty-library variant offers it —
   *  a query that matched nothing is not evidence about the scan. */
  readonly onOpenScanSummary: (() => void) | null;
}

export function EmptyState(props: EmptyStateProps): ReactElement {
  const { model } = props;
  const empty = model.variant === 'noLibrary';
  return (
    <div className="cdt-shelf-empty">
      <p className="cdt-shelf-empty-heading">{model.heading}</p>
      <p className="cdt-shelf-empty-reason">{model.reason}</p>
      <div className="cdt-shelf-empty-actions">
        {model.action === null ? null : (
          <button
            type="button"
            className="cdt-shelf-empty-action"
            onClick={empty ? props.onAddScanRoot : props.onClearQuery}
          >
            {model.action}
          </button>
        )}
        {empty && props.onOpenScanSummary !== null ? (
          <button
            type="button"
            className="cdt-shelf-empty-action"
            onClick={props.onOpenScanSummary}
          >
            {SCAN_PROBLEMS_LABEL}
          </button>
        ) : null}
      </div>
    </div>
  );
}
