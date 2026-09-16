/**
 * §8.5's page. Two tabs, a 268px hero column and the Overview column beside it.
 *
 * §11.7: each context owns its keys exclusively — this page binds `←`/`→` and `Esc` and nothing
 * else, and it asks the one key table rather than matching keys itself. The shelf's grid handler
 * must not run here, which §8.5.1 secures from the other side by unmounting the shelf whenever a
 * full-screen flow opens.
 */
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactElement,
} from 'react';

import type {
  LocationDetail,
  LocationId,
  ProjectDetail,
  ProjectId,
} from '../../generated/protocol';
import { resolveKey, type KeyEventLike } from '../keyboard/contexts';
import { ActivityTab } from './activity/ActivityTab';
import { useProjectPageDeps } from './deps';
import { HeroTile } from './hero/HeroTile';
import { Identity } from './Identity';
import { LocationsPanel } from './locations/LocationsPanel';
import { cascadeDelay } from './motion';
import { NotePanel } from './note/NotePanel';
import { Rail } from './rail/Rail';
import { ReadmePanel } from './readme/ReadmePanel';
import { RoastNote } from './RoastNote';
import { BASE_PROJECT_TABS, fallbackTab, nextTab, tabsFor, type ProjectTab } from './tabs';
import { useProjectDetail } from './useProjectDetail';

export const PROJECT_PAGE_ROOT_CLASS = 'cp-page';

export interface ProjectPageProps {
  projectId: ProjectId;
  onBack: () => void;
  onOpenProject: (id: ProjectId) => void;
  /**
   * §10.5a's boundary, which is a library-wide fact and not part of one project's payload, so it
   * arrives from whoever mounts the page. Absent means first run has not finished, under which
   * §10.5a says nothing is new — so the hero draws no `NEW` rather than guessing one.
   */
  firstRunCompletedAt?: number | null;
}

export function primaryLocation(detail: ProjectDetail): LocationDetail | null {
  return detail.locations.find((l) => l.isPrimary) ?? detail.locations[0] ?? null;
}

/**
 * §5.6 phrases its sentence from the location the page is showing — the one PLAY launches — so
 * the page holds the choice and defaults it to the primary. An id that is no longer in the set
 * falls back rather than leaving the page describing nothing.
 */
export function shownLocation(
  detail: ProjectDetail,
  shownId: LocationId | null,
): LocationDetail | null {
  if (shownId !== null) {
    const held = detail.locations.find((l) => l.location.id === shownId);
    if (held !== undefined) return held;
  }
  return primaryLocation(detail);
}

function asKeyEvent(event: ReactKeyboardEvent<HTMLDivElement>): KeyEventLike {
  const target = event.target;
  return {
    key: event.key,
    code: event.code,
    altKey: event.altKey,
    ctrlKey: event.ctrlKey,
    shiftKey: event.shiftKey,
    metaKey: event.metaKey,
    target:
      target instanceof HTMLElement
        ? { tagName: target.tagName, isContentEditable: target.isContentEditable }
        : null,
  };
}

// The exported symbol is `ProjectPageView`, because `ProjectPage` is the generated wire type for
// one page of the shelf's own query. The file keeps its name.
export function ProjectPageView({
  projectId,
  onBack,
  onOpenProject,
  firstRunCompletedAt = null,
}: ProjectPageProps): ReactElement {
  const { state, heroHash, redirectedTo, reload } = useProjectDetail(projectId);
  const [tab, setTab] = useState<ProjectTab>('overview');
  const [shownId, setShownId] = useState<LocationId | null>(null);
  const [roastsEnabled, setRoastsEnabled] = useState(true);
  const [pinnedOverride, setPinnedOverride] = useState<boolean | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const deps = useProjectPageDeps();

  useEffect(() => {
    if (redirectedTo !== null) onOpenProject(redirectedTo);
  }, [redirectedTo, onOpenProject]);

  useEffect(() => {
    rootRef.current?.focus();
  }, []);

  // A held location id belongs to one project's set. Following a merge, or opening a second
  // project from the palette, would otherwise carry a stranger's id into the fallback.
  useEffect(() => {
    setShownId(null);
    setPinnedOverride(null);
  }, [projectId]);

  // §11.3a's switch defaults **on**, so a settings read that has not landed yet must not silence
  // a note that would render, and a read that fails leaves the documented default in place.
  useEffect(() => {
    let live = true;
    deps
      .request('settings.get', {})
      .then((settings) => {
        if (live) setRoastsEnabled(settings.roastEnabled);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [deps]);

  const detailPinned = state.kind === 'ready' ? state.detail.row.isPinned : null;

  /**
   * The optimistic bit stands until the core's own answer agrees with it. Clearing it on any
   * arriving detail would let a reload already in flight when the press happened flip the mark
   * back; clearing it only on agreement means the mark moves once, in the direction the user
   * asked for. A refused write clears it below, so it cannot outlive the round trip.
   */
  useEffect(() => {
    if (pinnedOverride !== null && detailPinned === pinnedOverride) setPinnedOverride(null);
  }, [detailPinned, pinnedOverride]);

  const onTogglePin = useCallback(() => {
    if (detailPinned === null) return;
    const next = !(pinnedOverride ?? detailPinned);
    // §8.3 filters `is:pinned` client-side, so the projection flips first and
    // `projects/flags_changed` reconciles; a pin that waits for the round trip leaves the query
    // and the mark disagreeing.
    setPinnedOverride(next);
    deps
      .request('projects.setFlags', {
        id: projectId,
        isPinned: next,
        isArchived: null,
        isHidden: null,
      })
      .catch(() => {
        // The write did not land, or may not have: either way the core's last answer is the only
        // thing this page knows, so it goes back to drawing that.
        setPinnedOverride(null);
      });
  }, [deps, detailPinned, pinnedOverride, projectId]);

  const detail = state.kind === 'ready' ? state.detail : null;
  // §25.1: the mounted list is this project's. While the detail is still loading the bar draws
  // the two every project has, so it gains a tab rather than emptying and refilling.
  const tabs = detail === null ? BASE_PROJECT_TABS : tabsFor(detail);
  // A held tab that is no longer mounted — the remote binding went away while the page was open
  // — lands on `overview` rather than leaving the page pointing at a panel that is not there.
  const shownTab = fallbackTab(tabs, tab);

  const onKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      const hit = resolveKey('projectPage', asKeyEvent(event));
      if (hit === null) return;
      // `quickSwitch` crosses every context and belongs to whoever owns the palette.
      if (hit.action === 'quickSwitch') return;
      if (hit.preventDefault) event.preventDefault();
      if (hit.action === 'page.back') onBack();
      else if (hit.action === 'page.nextTab') setTab((current) => nextTab(tabs, current, 1));
      else if (hit.action === 'page.prevTab') setTab((current) => nextTab(tabs, current, -1));
    },
    [onBack, tabs],
  );

  const shown = detail === null ? null : shownLocation(detail, shownId);
  const primary = detail === null ? null : primaryLocation(detail);
  const pathDisplay = shown?.location.pathDisplay ?? '';

  return (
    <div
      ref={rootRef}
      className={PROJECT_PAGE_ROOT_CLASS}
      data-testid="cp-page"
      tabIndex={-1}
      onKeyDown={onKeyDown}
    >
      <div className="cp-bar">
        <button type="button" className="cp-back" onClick={onBack}>
          <span className="cp-back-arrow" aria-hidden="true">
            ←
          </span>
          SHELF
        </button>
        <span className="cp-bar-path" data-testid="cp-bar-path" title={pathDisplay}>
          {pathDisplay}
        </span>
        <span className="cp-bar-hint" aria-hidden="true">
          ESC · ←→
        </span>
        <div className="cp-tabs" role="tablist" aria-label="Project sections">
          {tabs.map((entry) => (
            <button
              key={entry.id}
              id={`cp-tab-${entry.id}`}
              type="button"
              role="tab"
              className="cp-tab"
              aria-selected={shownTab === entry.id}
              aria-controls="cp-tabpanel"
              tabIndex={shownTab === entry.id ? 0 : -1}
              onClick={() => {
                setTab(entry.id);
              }}
            >
              {entry.label}
            </button>
          ))}
        </div>
      </div>

      {state.kind === 'failed' ? (
        <div className="cp-body">
          <div className="cp-col-right">
            <p className="cp-notice">
              That project could not be read. It may have been moved since the last scan.
            </p>
            <div>
              <button type="button" className="cp-back" onClick={reload}>
                TRY AGAIN
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {detail !== null ? (
        <div className="cp-body">
          <div className="cp-col-left">
            <HeroTile
              row={detail.row}
              heroHash={heroHash}
              firstRunCompletedAt={firstRunCompletedAt}
              isPinned={pinnedOverride ?? detail.row.isPinned}
              onTogglePin={onTogglePin}
            />
            <Rail detail={detail} shown={shown} onChanged={reload} />
          </div>
          <div className="cp-col-right">
            <div className="cp-rise" style={{ animationDelay: cascadeDelay(0) }}>
              <Identity row={detail.row} />
              {/* §8.5.1 sits the note under the description, inside the identity block. */}
              <RoastNote
                detail={detail}
                shown={shown}
                primary={primary}
                roastsEnabled={roastsEnabled}
              />
            </div>
            {/*
             * §8.5.1 unmounts what is not on screen, and the same reasoning applies inside the
             * page: only the mounted tab's subtree exists. The cascade delay moved onto the
             * panels themselves, each of which carries §8.5.1's own beat.
             */}
            <div
              className="cp-tabpanel"
              id="cp-tabpanel"
              role="tabpanel"
              aria-labelledby={`cp-tab-${shownTab}`}
              data-testid="cp-tabpanel"
              data-tab={shownTab}
              data-shown-location={String(shown?.location.id ?? '')}
              data-primary-location={String(primary?.location.id ?? '')}
            >
              {shownTab === 'overview' ? (
                <>
                  <LocationsPanel
                    detail={detail}
                    shownId={shownId}
                    onShow={setShownId}
                    onChanged={reload}
                  />
                  <ReadmePanel readme={detail.readme} row={detail.row} now={deps.now()} />
                  <NotePanel
                    projectId={detail.row.id}
                    row={detail.row}
                    note={detail.notes}
                    onChanged={reload}
                  />
                </>
              ) : null}
              {shownTab === 'activity' ? <ActivityTab detail={detail} /> : null}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
