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
import { useProjectPageDeps } from './deps';
import { HeroTile } from './hero/HeroTile';
import { Identity } from './Identity';
import { cascadeDelay } from './motion';
import { RoastNote } from './RoastNote';
import { nextTab, PROJECT_TABS, type ProjectTab } from './tabs';
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

  const onKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      const hit = resolveKey('projectPage', asKeyEvent(event));
      if (hit === null) return;
      // `quickSwitch` crosses every context and belongs to whoever owns the palette.
      if (hit.action === 'quickSwitch') return;
      if (hit.preventDefault) event.preventDefault();
      if (hit.action === 'page.back') onBack();
      else if (hit.action === 'page.nextTab') setTab((current) => nextTab(current, 1));
      else if (hit.action === 'page.prevTab') setTab((current) => nextTab(current, -1));
    },
    [onBack],
  );

  const detail = state.kind === 'ready' ? state.detail : null;
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
          {PROJECT_TABS.map((entry) => (
            <button
              key={entry.id}
              id={`cp-tab-${entry.id}`}
              type="button"
              role="tab"
              className="cp-tab"
              aria-selected={tab === entry.id}
              aria-controls="cp-tabpanel"
              tabIndex={tab === entry.id ? 0 : -1}
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
            />
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
            <div
              className="cp-rise"
              style={{ animationDelay: cascadeDelay(1) }}
              id="cp-tabpanel"
              role="tabpanel"
              aria-labelledby={`cp-tab-${tab}`}
              data-testid="cp-tabpanel"
              data-tab={tab}
              data-shown-location={String(shown?.location.id ?? '')}
              data-primary-location={String(primary?.location.id ?? '')}
            />
          </div>
        </div>
      ) : null}
    </div>
  );
}
