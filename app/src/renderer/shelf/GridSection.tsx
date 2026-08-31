import { type CSSProperties, type ReactElement, type ReactNode, useEffect, useRef } from 'react';
import type { ProjectId, SessionRef } from '../../generated/protocol.js';
import { GRID_ROLE, GRID_ROW_ROLE } from '../a11y/names.js';
import { ProjectCard } from '../card/ProjectCard.js';
import type { GridFocus, GridState, MountWindow } from '../keyboard/gridNavigation.js';
import { mountedIndices } from '../keyboard/gridNavigation.js';
import type { GridMetrics, SectionExtent } from './measure.js';
import { GRID_ROW_GAP } from './measure.js';
import type { ShelfSection } from './page.js';

export interface GridRun {
  readonly rowIndex: number;
  readonly indices: readonly number[];
}

/**
 * The mounted set is not always contiguous: §11.7 keeps the focused project mounted wherever it
 * is, so a run per grid row and a spacer between runs is what a virtualized grid actually needs.
 */
export function gridRunsOf(indices: readonly number[], columns: number): readonly GridRun[] {
  const runs: { rowIndex: number; indices: number[] }[] = [];
  for (const index of [...indices].sort((a, b) => a - b)) {
    const rowIndex = Math.floor(index / columns);
    const last = runs.at(-1);
    if (last && last.rowIndex === rowIndex) last.indices.push(index);
    else runs.push({ rowIndex, indices: [index] });
  }
  return runs;
}

export interface GridSectionProps {
  readonly section: ShelfSection;
  readonly extent: SectionExtent;
  readonly metrics: GridMetrics;
  /**
   * 12b's window, and **`to` is inclusive** — `mountedIndices` reads it that way. This component
   * shifts it into the section's own index space and hands it straight to that function rather
   * than re-reading §11.7's rule, so the key handler and the DOM cannot disagree about what is
   * mounted.
   */
  readonly mountWindow: MountWindow;
  readonly focus: GridFocus;
  readonly density: number;
  /**
   * The shelf's shared instant, unix **seconds**. It has to *advance*: the card's own timer
   * repaints at the minute boundary but cannot invent a later second, so a constant here freezes
   * every bench row and every as-of clause on the grid.
   */
  readonly now: number;
  readonly firstRunCompletedAt: number | null;
  readonly selectedId: ProjectId | null;
  /** §7.8's live tile. A project with no open session is absent, never a zero-length one. */
  readonly sessions: ReadonlyMap<ProjectId, SessionRef>;
  /**
   * §11.6's flicker dips **one** card in the viewport, chosen above the grid because "at most one
   * card" is a shelf-level fact. `null` means no card is dipping.
   */
  readonly halo: { readonly projectId: ProjectId | null; readonly opacity: number } | null;
  readonly peek: ReactNode;
  readonly onPeekHeight?: (px: number) => void;
  readonly onActivate: (id: ProjectId) => void;
  readonly onOpen: (id: ProjectId) => void;
  readonly onTogglePin: (id: ProjectId) => void;
  readonly onStopSession: (id: ProjectId) => void;
}

export function GridSection(props: GridSectionProps): ReactElement {
  const {
    section,
    extent,
    metrics,
    mountWindow,
    focus,
    density,
    now,
    firstRunCompletedAt,
    selectedId,
    sessions,
    halo,
    peek,
    onPeekHeight,
    onActivate,
    onOpen,
    onTogglePin,
    onStopSession,
  } = props;
  const peekRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!onPeekHeight) return;
    onPeekHeight(peekRef.current?.offsetHeight ?? 0);
  }, [onPeekHeight, peek, selectedId]);

  if (extent.collapsed) return <div className="cdt-shelf-section-body" />;

  const state: GridState = {
    order: section.rows.map((row) => row.id),
    columns: metrics.columns,
  };
  // Shifted into the section's own index space and nothing else: `mountedIndices` already clamps
  // both ends to the sequence it was given, and a second clamp here is a second owner of that
  // rule, free to drift by one against the first.
  const local: MountWindow = {
    from: mountWindow.from - extent.firstIndex,
    to: mountWindow.to - extent.firstIndex,
  };
  const runs = gridRunsOf(mountedIndices(state, local, focus), metrics.columns);
  const pitch = metrics.rowHeight + GRID_ROW_GAP;
  const totalRows = Math.ceil(section.rows.length / metrics.columns);
  const selectedRun =
    selectedId === null
      ? -1
      : runs.findIndex((run) => run.indices.some((i) => section.rows[i]?.id === selectedId));

  const nodes: ReactNode[] = [];
  let drawnThrough = 0;
  runs.forEach((run, position) => {
    const gap = (run.rowIndex - drawnThrough) * pitch;
    if (gap > 0) {
      nodes.push(
        <div
          key={`spacer-${String(run.rowIndex)}`}
          className="cdt-shelf-spacer"
          style={{ height: `${String(gap)}px` }}
        />,
      );
    }
    drawnThrough = run.rowIndex + 1;
    nodes.push(
      <div
        key={`row-${String(run.rowIndex)}`}
        className="cdt-shelf-grid-row"
        role={GRID_ROW_ROLE}
        aria-rowindex={run.rowIndex + 1}
      >
        {run.indices.map((index) => {
          const row = section.rows[index];
          if (!row) return null;
          const id = row.id;
          return (
            <ProjectCard
              key={id}
              row={row}
              density={density}
              rendition="card"
              selected={selectedId === id}
              focused={focus.projectId === id}
              now={now}
              firstRunCompletedAt={firstRunCompletedAt}
              session={sessions.get(id) ?? null}
              haloOpacity={halo !== null && halo.projectId === id ? halo.opacity : 1}
              onActivate={() => {
                onActivate(id);
              }}
              onOpen={() => {
                onOpen(id);
              }}
              onTogglePin={() => {
                onTogglePin(id);
              }}
              onStopSession={() => {
                onStopSession(id);
              }}
            />
          );
        })}
      </div>,
    );
    // §8.4.1: Peek opens below the selected card. Full-row, so the grid stays a grid.
    if (peek && position === selectedRun) {
      nodes.push(
        <div key="peek" ref={peekRef} className="cdt-shelf-peek-slot">
          {peek}
        </div>,
      );
    }
  });

  const trailing = (totalRows - drawnThrough) * pitch;
  if (trailing > 0) {
    nodes.push(
      <div
        key="spacer-tail"
        className="cdt-shelf-spacer"
        style={{ height: `${String(trailing)}px` }}
      />,
    );
  }

  return (
    <div
      className="cdt-shelf-grid"
      role={GRID_ROLE}
      aria-rowcount={totalRows}
      aria-colcount={metrics.columns}
      style={{ '--tile': `${String(density)}px` } as CSSProperties}
    >
      {nodes}
    </div>
  );
}
