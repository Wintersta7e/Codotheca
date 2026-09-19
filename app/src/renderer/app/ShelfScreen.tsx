import {
  type ReactElement,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type {
  Peek,
  Problems,
  ProjectId,
  ScanStatus,
  SessionRef,
} from '../../generated/protocol.js';
import { parseQuery } from '../../shared/query/parse.js';
import type { KeyAction } from '../keyboard/contexts.js';
import {
  mountedIndices,
  moveFocus,
  NO_FOCUS,
  type GridDirection,
  type GridFocus,
  type GridState,
} from '../keyboard/gridNavigation.js';
import { flickerEligible, useFlicker } from '../motion/flicker.js';
import { allowsScheduledFrames, type ResolvedTier } from '../motion/tier.js';
import type { TransitionPhase } from '../motion/transition.js';
import { AttentionRow } from '../shelf/AttentionRow.js';
import { isCollapsed, toggled } from '../shelf/collapse.js';
import { shelfCounts } from '../shelf/counts.js';
import type { QueryContext } from '../shelf/evaluate.js';
import type { LibraryPresence } from '../shelf/EmptyState.js';
import { EraHeader } from '../shelf/EraHeader.js';
import { GridSection } from '../shelf/GridSection.js';
import { useShelfInstall } from '../shelf/useShelfInstall.js';
import { ListView } from '../shelf/ListView.js';
import type { Notice } from '../shelf/notice.js';
import { buildShelfPage } from '../shelf/page.js';
import { PeekPanel } from '../shelf/Peek.js';
import { ReferenceTail } from '../shelf/ReferenceTail.js';
import type { ShelfRow } from '../shelf/row.js';
import { Shelf } from '../shelf/Shelf.js';
import { useVirtualizer } from '../shelf/useVirtualizer.js';
import { SORT_LABELS, offeredSorts, resolveSort, type ShelfView } from '../shelf/viewState.js';
import type { AppDeps } from './deps.js';
import { buildQueryContext } from './queryContext.js';

const UNKNOWN_SCAN: Pick<ScanStatus, 'running' | 'foundRepos' | 'problemCount'> = {
  running: false,
  foundRepos: 0,
  problemCount: null,
};

interface PeekAnswer {
  readonly projectId: ProjectId;
  readonly peek: Peek;
}

interface FlatEntry {
  readonly row: ShelfRow;
  readonly sectionId: string;
}

interface PeekMeasurement {
  readonly sectionId: string;
  readonly height: number;
}

export interface ShelfScreenProps {
  readonly deps: AppDeps;
  readonly rows: readonly ShelfRow[];
  /**
   * What the projection said, which `rows` alone cannot carry: this screen takes a row list and
   * an unread library arrives here as `[]`, indistinguishable from a library that is empty.
   */
  readonly library: LibraryPresence;
  /** §11.1's report, or `null` for *not read* — the shelf's link into the summary is offered
   *  from this and never from `scan.status`, which is a second reading of the same run. */
  readonly problems: Problems | null;
  readonly generation: number;
  readonly view: ShelfView;
  readonly onViewChange: (next: ShelfView) => void;
  readonly notices: readonly Notice[];
  readonly scan: Pick<ScanStatus, 'running' | 'foundRepos' | 'problemCount'> | null;
  readonly sessions: ReadonlyMap<ProjectId, SessionRef>;
  readonly firstRunCompletedAt: number | null;
  readonly tier: ResolvedTier;
  /**
   * §8.5.1's gesture. The shelf recedes while it is still the view on screen and returns holding
   * a landing state, so it needs the phase rather than a boolean: `landing` also names the tile
   * to unfold and the section whose header flares.
   */
  readonly phase?: TransitionPhase;
  readonly onOpenProject: (id: ProjectId) => void;
  readonly onOpenPalette: () => void;
  readonly onOpenSettings: () => void;
  readonly onOpenScanSummary: () => void;
  readonly onAddScanRoot: () => void;
  readonly savedChips?: ReactElement | null;
  /** §8.0's box, for the one section whose row is a card rather than a string (§1.4). */
  readonly renderNotice?: (notice: Notice, dismiss: () => void) => ReactNode;
}

function directionFor(action: KeyAction): GridDirection | null {
  switch (action) {
    case 'shelf.moveUp':
      return 'up';
    case 'shelf.moveDown':
      return 'down';
    case 'shelf.moveLeft':
      return 'left';
    case 'shelf.moveRight':
      return 'right';
    default:
      return null;
  }
}

/**
 * §8.4's PLAY from the grid is not wired, and nothing here fakes it.
 *
 * `projects.launch` takes `{projectId, locationId, targetId}` and a `ProjectRow` carries neither
 * of the last two — the wire type has no primary location and no default target — so the shelf
 * cannot compose the command at all. Rather than a control that does nothing, the double-click
 * and §11.7's `shelf.play` both open the project page, which holds the location its own PLAY
 * button launches. Recorded as a gap; the fix is a field on the row, not a second launch path
 * invented here.
 */

export function ShelfScreen(props: ShelfScreenProps): ReactElement {
  const {
    deps,
    rows,
    generation,
    view,
    onViewChange,
    sessions,
    firstRunCompletedAt,
    tier,
    onOpenProject,
  } = props;
  const scrollRef = useRef<HTMLDivElement>(null);
  const desiredColumn = useRef(0);
  const [now, setNow] = useState(() => deps.now());
  const [peekOpen, setPeekOpen] = useState(false);
  const [peekAnswer, setPeekAnswer] = useState<PeekAnswer | null>(null);
  const [peekMeasurement, setPeekMeasurement] = useState<PeekMeasurement | null>(null);
  const [flickerSeed] = useState(() => Math.trunc(deps.nowMs()));
  const readNow = deps.now;
  const request = deps.request;
  // [p2] §24.3d's other mount point. The map is owned here, beside `sessions`, and each tile
  // asks for its own preview as it mounts.
  const install = useShelfInstall(deps);

  useEffect(() => {
    const advance = (): void => {
      setNow(readNow());
    };
    advance();
    const interval = window.setInterval(advance, 1000);
    return () => {
      window.clearInterval(interval);
    };
  }, [readNow]);

  const queryContext = useMemo<QueryContext>(
    () => buildQueryContext({ rows, now, firstRunCompletedAt }),
    [rows, now, firstRunCompletedAt],
  );
  // [p3] §35.5. The key is offered only when a row in the projection carries a reading, and a
  // stored key that cannot be honoured resolves **here**, where the comparator is chosen.
  // `view.sort` is not rewritten: `patchFor` issues a `view.set` only when `prev.sort !==
  // next.sort`, so leaving the stored value alone is the implementation of *nothing is written
  // back*, and the key returns the moment a reading exists.
  const offered = useMemo(() => offeredSorts(queryContext.capabilities), [queryContext]);
  const sort = resolveSort(view.sort, offered);
  const page = useMemo(
    () =>
      buildShelfPage({
        rows,
        query: view.query,
        sort,
        now,
        generation,
        ctx: queryContext,
      }),
    [generation, now, queryContext, rows, sort, view.query],
  );
  const counts = useMemo(() => shelfCounts(rows, page.matched), [page.matched, rows]);
  const flatEntries = useMemo<readonly FlatEntry[]>(
    () =>
      page.sections.flatMap((section) =>
        section.rows.map((row) => ({ row, sectionId: section.id })),
      ),
    [page.sections],
  );
  const flatRows = useMemo(() => flatEntries.map((entry) => entry.row), [flatEntries]);
  const queryIsEmpty = view.query.length === 0;
  const selectedSectionId =
    page.sections.find((section) => section.rows.some((row) => row.id === view.selectedProjectId))
      ?.id ?? null;
  const peekExtra =
    peekOpen &&
    selectedSectionId !== null &&
    peekMeasurement?.sectionId === selectedSectionId &&
    peekMeasurement.height > 0
      ? peekMeasurement
      : null;
  const virtual = useVirtualizer({
    page,
    tile: view.density,
    collapse: view.collapsed,
    queryIsEmpty,
    focusedProjectId: view.selectedProjectId,
    peekExtra,
    scrollRef,
  });
  const focus: GridFocus = {
    projectId: view.selectedProjectId,
    desiredColumn: desiredColumn.current,
  };
  const gridState = useMemo<GridState>(
    () => ({
      order: flatRows.map((row) => row.id),
      columns: virtual.metrics.columns,
    }),
    [flatRows, virtual.metrics.columns],
  );

  useEffect(() => {
    if (!peekOpen || view.selectedProjectId === null) return undefined;
    const projectId = view.selectedProjectId;
    let live = true;
    void request('projects.peek', { id: projectId }).then(
      (peek) => {
        if (live) setPeekAnswer({ projectId, peek });
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [peekOpen, request, view.selectedProjectId]);

  const rowsById = useMemo(() => new Map(rows.map((row) => [row.id, row] as const)), [rows]);
  const togglePin = useCallback(
    (projectId: ProjectId): void => {
      const row = rowsById.get(projectId);
      if (row === undefined) return;
      void request('projects.setFlags', {
        id: projectId,
        isPinned: !row.isPinned,
        isArchived: null,
        isHidden: null,
      }).then(
        () => undefined,
        () => undefined,
      );
    },
    [request, rowsById],
  );
  const stopSession = useCallback(
    (projectId: ProjectId): void => {
      const session = sessions.get(projectId);
      if (session === undefined) return;
      void request('session.stop', { id: session.id }).then(
        () => undefined,
        () => undefined,
      );
    },
    [request, sessions],
  );

  const onKeyAction = useCallback(
    (action: KeyAction): void => {
      const direction = directionFor(action);
      if (direction !== null) {
        const next = moveFocus(
          gridState,
          {
            projectId: view.selectedProjectId,
            desiredColumn: desiredColumn.current,
          },
          direction,
        );
        desiredColumn.current = next.desiredColumn;
        if (next.projectId !== view.selectedProjectId) {
          onViewChange({ ...view, selectedProjectId: next.projectId });
        }
        return;
      }

      const projectId = view.selectedProjectId;
      if (action === 'shelf.closePeek') {
        setPeekOpen(false);
      } else if (action === 'shelf.peek' && projectId !== null) {
        setPeekOpen((current) => !current);
      } else if ((action === 'shelf.openPage' || action === 'shelf.play') && projectId !== null) {
        onOpenProject(projectId);
      } else if (action === 'shelf.togglePin' && projectId !== null) {
        togglePin(projectId);
      }
    },
    [gridState, onOpenProject, onViewChange, togglePin, view],
  );

  const onPeekHeight = useCallback(
    (height: number): void => {
      if (selectedSectionId === null) return;
      setPeekMeasurement((current) => {
        if (current?.sectionId === selectedSectionId && current.height === height) return current;
        return { sectionId: selectedSectionId, height };
      });
    },
    [selectedSectionId],
  );

  const scheduledFrames = allowsScheduledFrames(tier);
  const collapsedSectionIds = useMemo(
    () => new Set(virtual.extents.filter((extent) => extent.collapsed).map((extent) => extent.id)),
    [virtual.extents],
  );
  const flickerCandidates = useMemo(() => {
    if (!scheduledFrames || view.viewMode !== 'grid') return [];
    return mountedIndices(gridState, virtual.mountWindow, NO_FOCUS).flatMap((index) => {
      const entry = flatEntries[index];
      if (
        entry === undefined ||
        collapsedSectionIds.has(entry.sectionId) ||
        !flickerEligible(entry.row)
      ) {
        return [];
      }
      return [entry.row.id];
    });
  }, [
    collapsedSectionIds,
    flatEntries,
    gridState,
    scheduledFrames,
    view.viewMode,
    virtual.mountWindow,
  ]);
  const flicker = useFlicker(flickerCandidates, {
    tier,
    seed: flickerSeed,
    monotonicMs: deps.nowMs,
  });
  const halo =
    scheduledFrames &&
    flicker.projectId !== null &&
    flicker.opacity < 1 &&
    flickerCandidates.includes(flicker.projectId)
      ? flicker
      : null;
  const resolvedPeek = peekAnswer?.projectId === view.selectedProjectId ? peekAnswer.peek : null;
  const peek = peekOpen ? <PeekPanel peek={resolvedPeek} now={now} tier={tier} /> : null;

  // §8.5.1's landing state: the tile you left unfolds, its neighbours ripple outward from it, and
  // the header of the section it sits in flares — so you can see which shelf you came back to.
  // Scoped to that one section, exactly as the handoff prototype scopes it.
  const landingId = props.phase?.kind === 'landing' ? props.phase.id : null;

  const grid = (
    <>
      <div style={{ minHeight: virtual.canvasHeight }}>
        {page.sections.map((section, index) => {
          const extent = virtual.extents[index];
          if (extent === undefined) return null;
          const collapsed = isCollapsed(
            view.collapsed,
            section.id,
            section.order,
            page.renderedTotal,
            queryIsEmpty,
          );
          const ownsPeek = peekOpen && selectedSectionId === section.id;
          return (
            <section className="cdt-shelf-section" key={section.id}>
              <EraHeader
                section={section}
                flaring={landingId !== null && section.rows.some((row) => row.id === landingId)}
                collapsed={collapsed}
                onToggle={() => {
                  onViewChange({
                    ...view,
                    collapsed: toggled(view.collapsed, section.id, !collapsed),
                  });
                }}
              />
              <GridSection
                section={section}
                extent={extent}
                metrics={virtual.metrics}
                mountWindow={virtual.mountWindow}
                focus={focus}
                density={view.density}
                now={now}
                firstRunCompletedAt={firstRunCompletedAt}
                selectedId={view.selectedProjectId}
                phase={props.phase}
                sessions={sessions}
                halo={halo}
                peek={ownsPeek ? peek : null}
                {...(ownsPeek ? { onPeekHeight } : {})}
                onActivate={onOpenProject}
                onOpen={onOpenProject}
                onTogglePin={togglePin}
                onStopSession={stopSession}
                installPreviews={install.previews}
                onNeedInstallPreview={install.need}
                onInstall={install.start}
                onOpenUpgrade={props.onOpenSettings}
              />
            </section>
          );
        })}
      </div>
      <ReferenceTail rows={page.reference} onOpen={onOpenProject} />
    </>
  );

  return (
    <Shelf
      view={view}
      page={page}
      counts={counts}
      phase={props.phase}
      notices={props.notices}
      scan={props.scan ?? UNKNOWN_SCAN}
      library={props.library}
      offeredSorts={offered}
      problems={props.problems}
      {...(props.renderNotice === undefined ? {} : { renderNotice: props.renderNotice })}
      now={now}
      peekOpen={peekOpen}
      onViewChange={onViewChange}
      onKeyAction={onKeyAction}
      onOpenPalette={props.onOpenPalette}
      onOpenSettings={props.onOpenSettings}
      onScan={() => {
        void request('scan.start', { full: false }).then(
          () => undefined,
          () => undefined,
        );
      }}
      onOpenScanSummary={props.onOpenScanSummary}
      onAddScanRoot={props.onAddScanRoot}
      scrollRef={scrollRef}
    >
      <AttentionRow
        rows={rows}
        ctx={queryContext}
        counts={counts}
        sortLabel={SORT_LABELS[sort].toUpperCase()}
        query={view.query}
        onQuery={(query) => {
          onViewChange({ ...view, query, ast: parseQuery(query) });
        }}
        savedChips={props.savedChips ?? null}
      />
      <div className="cdt-shelf-body">
        {view.viewMode === 'grid' ? (
          grid
        ) : (
          <ListView
            rows={flatRows}
            now={now}
            firstRunCompletedAt={firstRunCompletedAt}
            selectedId={view.selectedProjectId}
            peek={peek}
            onActivate={onOpenProject}
            onOpen={onOpenProject}
          />
        )}
      </div>
    </Shelf>
  );
}
