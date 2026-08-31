import { useMemo } from 'react';
import type { CSSProperties, ReactElement } from 'react';
import { effectiveQueryText } from '../../shared/query/format.js';
import { parseQuery } from '../../shared/query/parse.js';
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
}

export function AttentionRow(props: AttentionRowProps): ReactElement {
  const { rows, ctx, counts, sortLabel, query, onQuery } = props;
  const chipCounts = useMemo(() => attentionCounts(rows, ctx), [ctx, rows]);
  const running = useMemo(() => effectiveQueryText(parseQuery(query)), [query]);

  return (
    <div className="cdt-attention-row">
      {ATTENTION_CHIPS.map((chip) => {
        const pressed = running === effectiveQueryText(parseQuery(chip.query));
        return (
          <button
            type="button"
            className="cdt-attention-chip"
            key={chip.id}
            aria-pressed={pressed}
            style={{ '--cdt-chip-accent': chip.accent } as CSSProperties}
            onClick={() => {
              onQuery(chip.query);
            }}
          >
            <span className="cdt-attention-count">{chipCounts[chip.id]}</span>
            <span>
              <span className="cdt-attention-label">{chip.label}</span>
              <span className="cdt-attention-sub">{chip.sub}</span>
            </span>
          </button>
        );
      })}
      <div className="cdt-attention-tail">
        <span className="cdt-attention-headline">{headlineText(counts, sortLabel)}</span>
        <span className="cdt-attention-hint">{SPACE_PEEK_HINT}</span>
      </div>
    </div>
  );
}
