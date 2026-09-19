import type { CSSProperties, ReactElement, ReactNode } from 'react';
import type { ProjectId } from '../../generated/protocol.js';
import { conditionDotName } from '../a11y/names.js';
import { rungFor, scoreText } from '../card/completion.js';
import { conditionDot } from '../derive/condition.js';
import { token } from '../theme/tokens.js';
import { formatTrackedBytes } from '../format/size.js';
import { LIST_CHIP_COLUMN_PX, listChipsLabel, listRowChips } from './chips.js';
import type { ShelfRow } from './row.js';

export interface ListColumn {
  readonly index: number;
  readonly key: string;
  /**
   * [p3] §31.7 fills the two reserved columns, and **both gain a header cell**: an unlabelled
   * column carrying a value is a value with no name. The reserved widths are unchanged, which is
   * what makes this a fill rather than a re-cut.
   */
  readonly header: string | null;
  readonly widthPx: number | null;
}

export const LIST_COLUMNS: readonly ListColumn[] = [
  { index: 1, key: 'condition', header: null, widthPx: null },
  { index: 2, key: 'rank', header: 'RANK', widthPx: 16 },
  { index: 3, key: 'name', header: 'NAME', widthPx: null },
  { index: 4, key: 'language', header: 'LANG', widthPx: 38 },
  { index: 5, key: 'branch', header: 'BRANCH', widthPx: 58 },
  { index: 6, key: 'size', header: 'SIZE', widthPx: 52 },
  { index: 7, key: 'score', header: 'SCORE', widthPx: 44 },
  { index: 8, key: 'chips', header: null, widthPx: LIST_CHIP_COLUMN_PX },
];

export interface ListViewProps {
  readonly rows: readonly ShelfRow[];
  readonly now: number;
  readonly firstRunCompletedAt: number | null;
  readonly selectedId: ProjectId | null;
  readonly peek: ReactNode;
  readonly onActivate: (id: ProjectId) => void;
  readonly onOpen: (id: ProjectId) => void;
}

export function ListView(props: ListViewProps): ReactElement {
  const { rows, now, firstRunCompletedAt, selectedId, peek, onActivate, onOpen } = props;
  return (
    <div className="cdt-list" role="table">
      <div className="cdt-list-header" role="row">
        {LIST_COLUMNS.filter((column) => column.header !== null).map((column) => (
          <span key={column.key} className={`cdt-list-c${String(column.index)}`}>
            {column.header}
          </span>
        ))}
      </div>
      {rows.map((row) => {
        const id = row.id;
        const dot = conditionDot({
          signal: row.conditionSignal,
          isReference: row.isReference,
          isArchived: row.isArchived,
        });
        const dotName = conditionDotName(row.conditionSignal);
        const chips = listRowChips(row, now, firstRunCompletedAt);
        const rung = rungFor({
          completionLit: row.completionLit,
          completionApplicable: row.completionApplicable,
          isReference: row.isReference,
          hasWorkingCopy: row.primaryLocation !== null,
          isArchived: row.isArchived,
        });
        const score = scoreText(row.completionLit, row.completionApplicable);
        const node = (
          <div
            key={`row-${String(id)}`}
            role="row"
            className="cdt-list-row"
            aria-selected={selectedId === id}
            aria-label={[row.name, listChipsLabel(chips)].filter(Boolean).join(' — ')}
            onDoubleClick={() => onActivate(id)}
            onClick={() => onOpen(id)}
          >
            <span className="cdt-list-c1">
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
            </span>
            {/* [p3] §31.7's rank mark. `null` renders no node, which is the same rule the
                grid card's rank slot already carries: a dash in a column that could have been
                empty is a claim with no measurement behind it. */}
            <span className="cdt-list-c2">
              {rung === null ? null : (
                <span
                  className="cdt-list-rank"
                  aria-hidden="true"
                  style={{ color: token(rung.inkToken) } as CSSProperties}
                />
              )}
            </span>
            <span className="cdt-list-c3">{row.name}</span>
            <span className="cdt-list-c4">{row.primaryLanguage ?? ''}</span>
            <span className="cdt-list-c5">{row.branch ?? ''}</span>
            <span className="cdt-list-c6">
              {row.sizeTrackedBytes === null ? '—' : formatTrackedBytes(row.sizeTrackedBytes)}
            </span>
            {/* [p3] §31.7: `<lit>/<evaluable>` in tier ink, **never a percentage and never a
                bare numerator**. `null` is uncomputed and renders nothing. */}
            <span
              className="cdt-list-c7"
              style={(rung === null ? {} : { color: token(rung.inkToken) }) as CSSProperties}
            >
              {score}
            </span>
            <span className="cdt-list-c8">
              {chips.map((chip) => (
                <span className="cdt-chip" data-chip={chip.id} key={chip.id}>
                  {chip.text}
                </span>
              ))}
            </span>
          </div>
        );

        if (peek && selectedId === id) {
          return [
            node,
            <div key={`peek-${String(id)}`} className="cdt-shelf-peek-slot">
              {peek}
            </div>,
          ];
        }
        return node;
      })}
    </div>
  );
}
