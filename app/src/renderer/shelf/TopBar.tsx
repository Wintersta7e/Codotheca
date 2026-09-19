import type { ReactElement } from 'react';
import type { ScanStatus, SortKey, ViewMode } from '../../generated/protocol.js';
import { QUICK_SWITCH_CHORD } from '../keyboard/contexts.js';
import { DensityControl } from './DensityControl.js';
import type { QueryFieldModel } from './QueryField.js';
import { QueryFieldView } from './QueryField.js';
// R19: `ShedLevel`, `SHED_ORDER`, `SHED_WIDTHS` and `shedLevelFor` are declared in
// `useShedLevel.ts` and imported here, never re-exported from this file. This component imports
// that module, so exporting its table back out of the component is a cycle around a constant.
import { shedClassName, shedLevelFor } from './useShedLevel.js';
import type { ShelfView } from './viewState.js';
import { SORT_KEYS, SORT_LABELS, nextSort, resolveSort } from './viewState.js';

/** 40px, not the design prose's 42: the prototype renders 40 plus a 1px rule, and the handoff's
 *  own Fidelity clause makes the prototype authoritative where the two differ. */
export const TOP_BAR_HEIGHT_PX = 40 as const;

/** Measured by `app/e2e/topbar-floor.spec.ts`, not asserted here: the narrowest width at which
 *  the fully shed bar still fits at the widest sort label, with the query field at its own 80px
 *  floor. One pixel below it the row runs past the bar and the right-hand control is clipped.
 *
 *  [p3] ~~`508`~~ re-measured at `NEEDS ATTENTION`, which §35.2 made the widest value the control
 *  can render — `103.16px` against `LAST TOUCHED`'s `84.64px`, and shed level 2 renders the value
 *  alone. The harness derives the widest variant by measuring every one of them, so this moves
 *  again on its own the next time a label does. */
export const TOP_BAR_FLOOR_PX = 526;

export const WORDMARK = 'CODOTHECA' as const;

const SORT_KEY_LABEL = 'SORT';
const SWITCH_KEY_LABEL = 'SWITCH';
const GRID_LABEL = 'GRID';
const LIST_LABEL = 'LIST';
const SCAN_IDLE_LABEL = 'SCAN';
const SCAN_RUNNING_LABEL = 'SCANNING';
const SETTINGS_NAME = 'Settings';
const SWITCH_NAME = 'Quick switch';

export interface TopBarProps {
  readonly view: ShelfView;
  readonly field: QueryFieldModel;
  readonly scan: Pick<ScanStatus, 'running' | 'foundRepos'>;
  readonly barWidth: number;
  /**
   * [p3] §35.5's offered set, from `offeredSorts(projectionCapabilities(rows))`. The bar renders
   * the **resolved** key and cycles within this list, and `view.sort` is never rewritten — a
   * stored preference is not deleted because today's library cannot honour it.
   *
   * The default is the whole cycle, which is what a caller with no projection means: this
   * component's own test and `app/e2e/topbar-floor.spec.ts` render the bar with no rows behind it.
   */
  readonly offeredSorts?: readonly SortKey[];
  readonly onQueryChange: (text: string) => void;
  readonly onSortChange: (sort: SortKey) => void;
  readonly onDensityChange: (density: number) => void;
  readonly onViewModeChange: (mode: ViewMode) => void;
  readonly onScan: () => void;
  readonly onOpenScanSummary: () => void;
  readonly onOpenPalette: () => void;
  readonly onOpenSettings: () => void;
}

/**
 * Block 1: the wordmark, seven slots and §8.0a's stated shed order.
 *
 * Two of the prototype's controls are fixtures with no §2.4 command behind them; their slots are
 * re-let rather than lost — `SCAN` takes `FIRST RUN`'s and `DENSITY` takes `LIBRARY`'s.
 *
 * Every label a user must read to work a control sits at `--text-3` or lighter, and all of that
 * is `shelf.css`: this module writes no colour.
 */
export function TopBar(props: TopBarProps): ReactElement {
  const shed = shedLevelFor(props.barWidth);
  const { view, scan } = props;
  const offered = props.offeredSorts ?? SORT_KEYS;
  const sort = resolveSort(view.sort, offered);
  const sortLabel = SORT_LABELS[sort];

  return (
    <div className={['cdt-shelf-bar', shedClassName(shed)].filter(Boolean).join(' ')}>
      <div className="cdt-shelf-wordmark">
        <span className="cdt-shelf-mark" aria-hidden="true">
          <i className="cdt-shelf-mark-bar" />
          <i className="cdt-shelf-mark-bar is-short" />
          <i className="cdt-shelf-mark-bar" />
        </span>
        {shed < 3 ? <span className="cdt-shelf-wordmark-text">{WORDMARK}</span> : null}
      </div>

      <QueryFieldView
        text={view.query}
        ast={view.ast}
        model={props.field}
        onQueryChange={props.onQueryChange}
      />

      <button
        type="button"
        data-slot="sort"
        className="cdt-shelf-control cdt-shelf-control-cycles"
        aria-label={`Sort: ${sortLabel.toLowerCase()}`}
        onClick={() => {
          props.onSortChange(nextSort(sort, offered));
        }}
      >
        {shed < 2 ? <span className="cdt-shelf-control-key">{SORT_KEY_LABEL}</span> : null}
        <span className="cdt-shelf-control-value">{sortLabel.toUpperCase()}</span>
      </button>

      <button
        type="button"
        data-slot="scan"
        className="cdt-shelf-control cdt-shelf-scan"
        onClick={scan.running ? props.onOpenScanSummary : props.onScan}
      >
        {scan.running ? (
          <>
            <span className="cdt-shelf-control-key">{SCAN_RUNNING_LABEL}</span>
            <span className="cdt-shelf-control-value">{String(scan.foundRepos)}</span>
          </>
        ) : (
          <span className="cdt-shelf-control-value">{SCAN_IDLE_LABEL}</span>
        )}
      </button>

      <div className="cdt-shelf-seg">
        <button
          type="button"
          data-slot="grid"
          className="cdt-shelf-seg-item"
          aria-pressed={view.viewMode === 'grid'}
          onClick={() => {
            props.onViewModeChange('grid');
          }}
        >
          {GRID_LABEL}
        </button>
        <button
          type="button"
          data-slot="list"
          className="cdt-shelf-seg-item"
          aria-pressed={view.viewMode === 'list'}
          onClick={() => {
            props.onViewModeChange('list');
          }}
        >
          {LIST_LABEL}
        </button>
      </div>

      {shed < 1 ? (
        <button
          type="button"
          data-slot="switch"
          className="cdt-shelf-control"
          aria-label={SWITCH_NAME}
          onClick={props.onOpenPalette}
        >
          <span className="cdt-shelf-control-key">{SWITCH_KEY_LABEL}</span>
          <span className="cdt-shelf-control-value">{QUICK_SWITCH_CHORD.toUpperCase()}</span>
        </button>
      ) : null}

      <DensityControl
        density={view.density}
        viewMode={view.viewMode}
        showKey={shed < 2}
        onCycle={props.onDensityChange}
      />

      <button
        type="button"
        data-slot="settings"
        className="cdt-shelf-glyph"
        aria-label={SETTINGS_NAME}
        onClick={props.onOpenSettings}
      >
        <span className="cdt-shelf-glyph-mark" aria-hidden="true" />
      </button>
    </div>
  );
}
