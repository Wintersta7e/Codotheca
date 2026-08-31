import { useMemo } from 'react';
import type { CSSProperties, ReactElement, ReactNode } from 'react';
import { effectiveQueryText } from '../../shared/query/format.js';
import { parseQuery } from '../../shared/query/parse.js';
import { ATTENTION_CHIPS, attentionCounts, headlineText } from './counts.js';
import type { ShelfCounts } from './counts.js';
import type { QueryContext } from './evaluate.js';
import type { ShelfRow } from './row.js';

/** §8.0b: `--text-5`'s stated envelope — a keyboard hint at 8px with .14em tracking. */
export const SPACE_PEEK_HINT = 'SPACE = PEEK';

export interface AttentionChipProps {
  readonly label: string;
  /**
   * `null` renders **no number at all**, not a zero. A broken saved query has not counted zero
   * projects; it has not counted. This is "never render unknown as zero" landing on the chip.
   */
  readonly count: number | null;
  /**
   * A node, not a string: a degraded saved query strikes through the terms it had to drop, in
   * place, inside the sub-line — which plain text cannot express.
   */
  readonly subLine: ReactNode;
  /** §8.0b's per-chip accent. `ALL`'s is `--text-2`, which is also the default. */
  readonly accent?: string;
  /** §8.0b defines no broken state; only the outline changes, to `--fail-hot`. */
  readonly broken?: boolean;
  readonly pressed: boolean;
  readonly onActivate: () => void;
}

export function AttentionChip(props: AttentionChipProps): ReactElement {
  const {
    label,
    count,
    subLine,
    accent = 'var(--text-2)',
    broken = false,
    pressed,
    onActivate,
  } = props;

  return (
    <button
      type="button"
      className={`cdt-attention-chip${broken ? ' cdt-attention-chip--broken' : ''}`}
      aria-pressed={pressed}
      style={{ '--cdt-chip-accent': accent } as CSSProperties}
      onClick={onActivate}
    >
      {count !== null ? <span className="cdt-attention-count">{count}</span> : null}
      <span>
        <span className="cdt-attention-label">{label}</span>
        <span className="cdt-attention-sub">{subLine}</span>
      </span>
    </button>
  );
}

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
        // `attentionCounts` fills every chip id, so `?? null` is unreachable for this row. If
        // that ever changes, rendering no number is the honest outcome.
        return (
          <AttentionChip
            key={chip.id}
            label={chip.label}
            count={chipCounts[chip.id] ?? null}
            subLine={chip.sub}
            accent={chip.accent}
            pressed={pressed}
            onActivate={() => {
              onQuery(chip.query);
            }}
          />
        );
      })}
      <div className="cdt-attention-tail">
        <span className="cdt-attention-headline">{headlineText(counts, sortLabel)}</span>
        <span className="cdt-attention-hint">{SPACE_PEEK_HINT}</span>
      </div>
    </div>
  );
}
