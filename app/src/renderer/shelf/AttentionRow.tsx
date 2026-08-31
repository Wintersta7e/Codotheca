import { useMemo } from 'react';
import type { ReactElement } from 'react';
import { effectiveQueryText } from '../../shared/query/format.js';
import { parseQuery } from '../../shared/query/parse.js';
import { AttentionChip } from './AttentionChip.js';
import { ATTENTION_CHIPS, attentionCounts, headlineText } from './counts.js';
import type { ShelfCounts } from './counts.js';
import type { QueryContext } from './evaluate.js';
import type { ShelfRow } from './row.js';

/** §8.0b: `--text-5`'s stated envelope — a keyboard hint at 8px with .14em tracking. */
export const SPACE_PEEK_HINT = 'SPACE = PEEK';

export interface AttentionRowProps {
  readonly rows: readonly ShelfRow[];
  readonly ctx: QueryContext;
  readonly counts: ShelfCounts;
  readonly sortLabel: string;
  readonly query: string;
  readonly onQuery: (query: string) => void;
  /**
   * §8.8's saved chips, appended after the four built-ins inside this same wrapping row. The row
   * owns the geometry; what goes in the tail is the collections module's, and this component
   * never counts one — a count here would be a second predicate beside §8.3a's one function.
   */
  readonly savedChips?: ReactElement | null;
}

export function AttentionRow(props: AttentionRowProps): ReactElement {
  const { rows, ctx, counts, sortLabel, query, onQuery } = props;
  const chipCounts = useMemo(() => attentionCounts(rows, ctx), [ctx, rows]);
  const running = useMemo(() => effectiveQueryText(parseQuery(query)), [query]);

  return (
    <div className="cdt-attention-row">
      {ATTENTION_CHIPS.map((chip) => (
        <AttentionChip
          key={chip.id}
          count={chipCounts[chip.id] ?? null}
          label={chip.label}
          subLine={chip.sub}
          accent={chip.accent}
          active={running === effectiveQueryText(parseQuery(chip.query))}
          onActivate={() => {
            onQuery(chip.query);
          }}
        />
      ))}
      {props.savedChips}
      <div className="cdt-attention-tail">
        <span className="cdt-attention-headline">{headlineText(counts, sortLabel)}</span>
        <span className="cdt-attention-hint">{SPACE_PEEK_HINT}</span>
      </div>
    </div>
  );
}
