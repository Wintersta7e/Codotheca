/**
 * The composition root.
 *
 * It provides the one injected door, runs the hooks, computes the route and renders it — and it
 * **holds no logic of its own**. Anything that wants a branch belongs in `app/route.ts` or in a
 * hook with its own test; the only state here is which overlay is open and which page, which
 * are facts about the window rather than decisions about the library.
 */
import { useCallback, useMemo, useState, type ReactElement } from 'react';

import type { LocationId, ProjectId } from '../generated/protocol';
import { AppDepsContext, createDefaultAppDeps, type AppDeps } from './app/deps';
import { FirstRunHost } from './app/FirstRunHost';
import { routeFor } from './app/route';
import { ShelfScreen } from './app/ShelfScreen';
import { SurfaceHost } from './app/SurfaceHost';
import { useCoreStatus } from './app/useCoreStatus';
import { useIdentity, identityNeedsConfirming } from './app/useIdentity';
import { useLibrary } from './app/useLibrary';
import { useNotices } from './app/useNotices';
import { useProblems } from './app/useProblems';
import { useScanStatus } from './app/useScanStatus';
import { useSessions } from './app/useSessions';
import { useSync } from './app/useSync';
import { useViewState } from './app/useViewState';
import { useResolvedTier, useTierOnDocument } from './motion/tier';
import { IdentityCard } from './firstrun/IdentityCard';
import { ProjectPageDepsContext } from './project/deps';
import { ProjectPageView } from './project/ProjectPage';
import { DEFAULT_DENSITY_PX } from './shelf/viewState';

/**
 * §8: the view is written back after the user stops moving, not on every keystroke. Held here
 * because the mount is what knows the write is a round trip to the core.
 */
export const VIEW_WRITE_DEBOUNCE_MS = 400;

export interface AppProps {
  /** Injected under test. In the product it is the preload bridge, built once. */
  readonly deps?: AppDeps;
}

export function App(props: AppProps = {}): ReactElement {
  const injected = props.deps;
  const deps = useMemo(() => injected ?? createDefaultAppDeps(), [injected]);

  const core = useCoreStatus(deps);
  const library = useLibrary(deps);
  const scan = useScanStatus(deps);
  const sessions = useSessions(deps);
  const [view, setView] = useViewState(deps, VIEW_WRITE_DEBOUNCE_MS);
  const problems = useProblems(deps, scan);
  // §1.4's set decides `authored_by_user` for every project, so confirming it changes the shelf
  // under the user: the library is re-read on the write rather than on the next launch.
  const identity = useIdentity(deps, library.reload);
  // [p2] §21's lane. One subscription, held here beside the others so the banner and the progress
  // line read the same payload rather than two.
  const sync = useSync(deps);

  const [openProjectId, setOpenProjectId] = useState<ProjectId | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [summaryOpen, setSummaryOpen] = useState(false);

  // §11.6 resolves `auto` here, where the media query and the compositor live. Software
  // compositing is the shell's finding and reaches the window as the tier it already resolved,
  // so there is nothing further for the renderer to detect.
  const tier = useResolvedTier(deps.effectsTier, false, false);
  // And it has to reach the document element, or the CSS reads the unresolved boot value forever.
  useTierOnDocument(tier);

  const [paletteNonce, setPaletteNonce] = useState(0);
  const openPalette = useCallback(() => {
    setPaletteNonce((n) => n + 1);
  }, []);
  const openScanSummary = useCallback(() => {
    setSummaryOpen(true);
  }, []);
  const openLog = useCallback(() => {
    void deps.reveal('log');
  }, [deps]);

  const notices = useNotices({
    degraded: core.degraded,
    gitVersion: core.gitVersion,
    // §11.2's five spawn sentences are plan 19's `coreFailure.ts` and do not exist. Nothing is
    // invented here, so a core that failed to spawn raises no notice — recorded, not papered.
    spawnFailure: null,
    problems: problems.problems,
    identityToConfirm: identityNeedsConfirming(identity.rows),
    // [p2] §21.10's banner, from the `sync` topic. `null` is *no sync failure*.
    sync: sync.notice,
    onOpenLog: openLog,
    onOpenScanSummary: openScanSummary,
  });

  const rows = library.rows;
  const route = routeFor({
    lane: core.lane,
    startupFailure: core.startupFailure,
    scan,
    // §11.2: a stored shelf that painted rows is proof a scan has already run.
    hasStoredShelf: rows !== null && rows.length > 0,
    openProjectId,
  });

  const openProject = useCallback((id: ProjectId) => {
    setOpenProjectId(id);
  }, []);
  const liveSessionProjectIds = useMemo(() => new Set(sessions.keys()), [sessions]);

  const showMe = useCallback(
    (query: string) => {
      // §10.4a: the turn's rung sets the query, expands every era section and resets density.
      setView({ ...view, query, collapsed: new Map(), density: DEFAULT_DENSITY_PX });
    },
    [setView, view],
  );

  const body =
    route.kind === 'project' ? (
      <ProjectPageView
        projectId={route.id}
        onBack={() => {
          setOpenProjectId(null);
        }}
        onOpenProject={openProject}
        firstRunCompletedAt={core.firstRunCompletedAt}
      />
    ) : (
      <ShelfScreen
        deps={deps}
        rows={rows ?? []}
        library={library.presence}
        problems={problems.problems}
        generation={library.generation}
        view={view}
        onViewChange={setView}
        notices={notices}
        scan={scan}
        sessions={sessions}
        firstRunCompletedAt={core.firstRunCompletedAt}
        tier={tier}
        onOpenProject={openProject}
        onOpenPalette={openPalette}
        onOpenSettings={() => {
          setSettingsOpen(true);
        }}
        onOpenScanSummary={openScanSummary}
        renderNotice={(notice, dismiss) =>
          notice.kind !== 'identity' || identity.rows === null ? null : (
            <IdentityCard
              rows={identity.rows}
              preview={identity.preview}
              onPreview={identity.askPreview}
              onConfirm={(emails) => {
                identity.confirm(emails);
                dismiss();
              }}
              onLeaveAsIs={dismiss}
            />
          )
        }
        onAddScanRoot={() => {
          void deps.pickRoot(false).then(
            (reply) => {
              if (reply.kind === 'added') library.reload();
            },
            () => undefined,
          );
        }}
      />
    );

  return (
    <AppDepsContext.Provider value={deps}>
      {/* One object, two contexts. `AppDeps` extends `ProjectPageDeps`, so the page's door is
          the same door and not a second construction of one. */}
      <ProjectPageDepsContext.Provider value={deps}>
        {route.kind !== 'failure' && (
          <FirstRunHost
            deps={deps}
            rows={rows ?? []}
            firstRunCompletedAt={core.firstRunCompletedAt}
            status={scan}
            hasStoredShelf={rows !== null && rows.length > 0}
            tier={tier}
            onOpenScanSummary={openScanSummary}
            onShowMe={showMe}
          >
            {body}
          </FirstRunHost>
        )}
        <SurfaceHost
          deps={deps}
          tier={tier}
          failure={route.kind === 'failure' ? route.fact : null}
          settingsOpen={settingsOpen}
          onCloseSettings={() => {
            setSettingsOpen(false);
          }}
          summaryOpen={summaryOpen}
          onCloseSummary={() => {
            setSummaryOpen(false);
          }}
          problems={problems.problems}
          onProblemsChanged={problems.reload}
          rows={rows ?? []}
          liveSessionProjectIds={liveSessionProjectIds}
          onOpenProject={openProject}
          openPaletteNonce={paletteNonce}
          onLaunch={(projectId: ProjectId, locationId: LocationId) => {
            // §4bis's five tiers are what `targets.list` answers with `resolved`, and
            // `projects.launch` needs the id it names. Nothing resolved is not an error to
            // invent a window for here — §11.5 owns that surface — so the launch simply does
            // not happen and the palette stays where it was.
            void deps.request('targets.list', { projectId, locationId }).then(
              (list) => {
                const targetId = list.resolved?.target.id;
                if (targetId === undefined) return;
                void deps.request('projects.launch', { projectId, locationId, targetId });
              },
              () => undefined,
            );
          }}
        />
      </ProjectPageDepsContext.Provider>
    </AppDepsContext.Provider>
  );
}
