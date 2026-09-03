import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, ReactElement } from 'react';
import type { LocationId, ProjectId, ProjectRow } from '../../generated/protocol.js';
import type { EffectsTier } from '../../shared/effectsTier.js';
// R12: 12b owns the accessibility names. `conditionDotName` is already the word-never-the-colour
// rule in code; a second copy beside the palette would be the duplication R12 exists to close.
import { conditionDotName } from '../a11y/names.js';
import { DOT_SIZE_PX, conditionDot } from '../derive/condition.js';
import { paletteRowDelayMs } from './motion.js';
import {
  PALETTE_UNAVAILABLE_TEXT,
  paletteCountText,
  paletteRowAction,
  selectPaletteRows,
} from './rows.js';
import { paletteSubLine, subLineInputFor } from './subline.js';
import './quickSwitch.css';

export const PALETTE_PLACEHOLDER = 'type a project name';
export const PALETTE_EMPTY_TEXT = 'No project matches that.';
export const PALETTE_FOOTER_HINTS = ['↑↓ MOVE', '↵ LAUNCH', '⇧↵ OPEN THE PAGE', 'ESC'] as const;

export interface QuickSwitchProps {
  readonly rows: readonly ProjectRow[];
  readonly query: string;
  readonly cursor: number;
  readonly liveSessionProjectIds: ReadonlySet<ProjectId>;
  readonly nowSecs: number;
  /** Already resolved: `auto` never reaches a component. */
  readonly effectsTier: EffectsTier;
  readonly jewelFor: (row: ProjectRow) => string;
  readonly onQueryChange: (value: string) => void;
  readonly onPoint: (index: number) => void;
  readonly onLaunch: (projectId: ProjectId, locationId: LocationId) => void;
  readonly onOpenPage: (projectId: ProjectId) => void;
  readonly onClose: () => void;
  readonly onKeyDown: (event: ReactKeyboardEvent<HTMLElement>) => void;
}

const OPTION_ID = (id: ProjectId): string => `qs-option-${String(id)}`;

function Row({
  row,
  index,
  selected,
  props,
  onCommit,
}: {
  readonly row: ProjectRow;
  readonly index: number;
  readonly selected: boolean;
  readonly props: QuickSwitchProps;
  readonly onCommit: (row: ProjectRow) => void;
}): ReactElement {
  const dot = conditionDot({
    signal: row.conditionSignal,
    isReference: row.isReference,
    isArchived: row.isArchived,
  });
  const dotName = conditionDotName(row.conditionSignal);
  const size = DOT_SIZE_PX.quickSwitch;
  const action = paletteRowAction(row);
  const style: CSSProperties & Record<string, string> = {
    '--cdt-qs-row-delay': `${String(paletteRowDelayMs(index, props.effectsTier))}ms`,
    '--cdt-qs-jewel': selected ? props.jewelFor(row) : 'transparent',
  };

  return (
    <div
      className="qs-row"
      id={OPTION_ID(row.id)}
      role="option"
      aria-selected={selected}
      style={style}
      onMouseEnter={() => {
        props.onPoint(index);
      }}
      onClick={() => {
        onCommit(row);
      }}
    >
      {/* Criterion 45b: the 17px leading slot takes §5.4a's dot and no rank glyph. Nothing at
          all when `condition_signal` is null — the slot's emptiness is the assertion, so the
          dot's name is an attribute rather than a text node. */}
      <span className="qs-dot-slot">
        {dot === null ? null : (
          <span
            className="qs-dot"
            role="img"
            {...(dotName === null ? {} : { 'aria-label': dotName })}
            style={{
              width: `${String(size)}px`,
              height: `${String(size)}px`,
              ...(dot.fill === null ? {} : { background: dot.fill }),
              ...(dot.ring === null ? {} : { border: dot.ring }),
              ...(dot.glow === null ? {} : { boxShadow: dot.glow }),
            }}
          />
        )}
      </span>
      <span className="qs-body">
        <span className="qs-name">{row.name}</span>
        <span className="qs-sub">
          {paletteSubLine(
            subLineInputFor(row, props.liveSessionProjectIds.has(row.id), props.nowSecs),
          )}
        </span>
      </span>
      {selected ? (
        action.kind === 'launch' ? (
          <span className="qs-act">{PALETTE_FOOTER_HINTS[1]}</span>
        ) : (
          <span className="qs-act qs-act--unavailable">{PALETTE_UNAVAILABLE_TEXT}</span>
        )
      ) : null}
    </div>
  );
}

export function QuickSwitch(props: QuickSwitchProps): ReactElement {
  const selection = selectPaletteRows(props.rows, props.query);
  const cursor = Math.min(props.cursor, Math.max(0, selection.rows.length - 1));
  const active = selection.rows[cursor];

  const commitLaunch = (row: ProjectRow): void => {
    const action = paletteRowAction(row);
    if (action.kind === 'launch') {
      props.onLaunch(row.id, action.locationId);
    } else {
      props.onOpenPage(row.id);
    }
    props.onClose();
  };

  return (
    <div
      className="qs-backdrop"
      onClick={props.onClose}
      onKeyDown={props.onKeyDown}
      role="presentation"
    >
      <div
        className="qs-panel"
        onClick={(event) => {
          event.stopPropagation();
        }}
        role="dialog"
        aria-modal="true"
        aria-label="Quick switch"
      >
        <div className="qs-bar">
          <span className="qs-caret" aria-hidden="true">
            ›
          </span>
          <input
            className="qs-input"
            role="combobox"
            aria-expanded="true"
            aria-controls="qs-listbox"
            aria-autocomplete="list"
            aria-label={PALETTE_PLACEHOLDER}
            {...(active === undefined ? {} : { 'aria-activedescendant': OPTION_ID(active.id) })}
            autoFocus
            value={props.query}
            placeholder={PALETTE_PLACEHOLDER}
            onChange={(event) => {
              props.onQueryChange(event.target.value);
            }}
          />
          <span className="qs-count">{paletteCountText(selection)}</span>
        </div>

        <div className="qs-list" id="qs-listbox" role="listbox" aria-label="Projects">
          {selection.rows.map((row, index) => (
            <Row
              key={String(row.id)}
              row={row}
              index={index}
              selected={index === cursor}
              props={props}
              onCommit={commitLaunch}
            />
          ))}
          {selection.rows.length === 0 ? (
            <div className="qs-empty">{PALETTE_EMPTY_TEXT}</div>
          ) : null}
        </div>

        <div className="qs-footer">
          <span className="qs-hint">{PALETTE_FOOTER_HINTS[0]}</span>
          <span className="qs-hint">{PALETTE_FOOTER_HINTS[1]}</span>
          <span className="qs-hint">{PALETTE_FOOTER_HINTS[2]}</span>
          <span className="qs-spacer" />
          <span className="qs-hint">{PALETTE_FOOTER_HINTS[3]}</span>
        </div>
      </div>
    </div>
  );
}
