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
  Settings,
} from '../../generated/protocol';
import { useInstallOffer } from '../install/useInstallOffer';
import { movesHealthReading, toSettingsPatch } from '../settings/Drawer';
import { resolveKey, type KeyEventLike } from '../keyboard/contexts';
import { ActivityTab } from './activity/ActivityTab';
import { BackupStateBlock } from './BackupState';
import { useProjectPageDeps } from './deps';
import { ConditionPanel } from './condition/ConditionPanel';
import { HeroTile } from './hero/HeroTile';
import { Identity } from './Identity';
import { LocationsPanel } from './locations/LocationsPanel';
import { cascadeDelay } from './motion';
import { NotePanel } from './note/NotePanel';
import { Rail } from './rail/Rail';
import { ReadmePanel } from './readme/ReadmePanel';
import { RemoteTab } from './remote/RemoteTab';
import { RoastNote } from './RoastNote';
import { CompletionChecklist } from './completion/CompletionChecklist';
import { DebtList } from './health/DebtList';
import { HealthTab } from './health/HealthTab';
import { BASE_PROJECT_TABS, fallbackTab, nextTab, tabsFor, type ProjectTab } from './tabs';
import { useUninstallOffer } from './uninstall/useUninstall';
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
  /**
   * §8.5.1: the page racks out before the shelf comes back. The gesture is owned above this
   * component because the shelf has to be restored 300 ms into a 340 ms animation this page is
   * still running — a page that unmounted itself at the end of its own animation would leave a
   * 40 ms hole, and one that unmounted at the start would never play it.
   */
  racking?: boolean;
  /**
   * [p2] §24.3d: on `private_needs_upgrade` the Install control states the consequence and offers
   * §20.3's upgrade, which lives in the settings drawer. The drawer is an overlay owned above
   * this page, so the gesture arrives from whoever mounts it — and without one the control
   * offers no upgrade rather than a switch that does nothing (§11.3a).
   */
  onOpenSettings?: () => void;
  /**
   * [p3] Moves each time a stored write `movesHealthReading` — the drawer's, or this page's own
   * grant, which reports to the same handler through `onHealthInputsChanged`. `settings.set`
   * raises no event, so the page re-reads its detail and its settings on this: a check switched
   * off takes its items off the list and the layers (§30.9), and the grant moves `todo_marker`.
   */
  healthInputsNonce?: number;
  /** [p3] The same handler the drawer reports to, for the tab's own grant. */
  onHealthInputsChanged?: (() => void) | undefined;
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
  racking = false,
  onOpenSettings,
  healthInputsNonce = 0,
  onHealthInputsChanged,
}: ProjectPageProps): ReactElement {
  const { state, heroHash, redirectedTo, reload } = useProjectDetail(projectId);
  const [tab, setTab] = useState<ProjectTab>('overview');
  const [shownId, setShownId] = useState<LocationId | null>(null);
  const [roastsEnabled, setRoastsEnabled] = useState(true);
  // [p3] §30.7 and R142: the `HEALTH` tab tells the two causes of `off` apart from two settings
  // fields, so it needs the whole `Settings` rather than one flag. `null` is *the read has not
  // landed*, under which no `off` check offers either control — never a guessed one.
  const [settings, setSettings] = useState<Settings | null>(null);
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
  // Re-read when a health input moves, so the HEALTH tab's two causes of `off` follow the drawer.
  useEffect(() => {
    let live = true;
    deps
      .request('settings.get', {})
      .then((loaded) => {
        if (!live) return;
        setRoastsEnabled(loaded.roastEnabled);
        setSettings(loaded);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [deps, healthInputsNonce]);

  // The mount already read the detail, so only a nonce that has moved since re-reads it.
  const seenHealthInputs = useRef(healthInputsNonce);
  useEffect(() => {
    if (seenHealthInputs.current === healthInputsNonce) return;
    seenHealthInputs.current = healthInputsNonce;
    reload();
  }, [healthInputsNonce, reload]);

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
  // §5.6: the page's own sentence is phrased from the copy it is showing, and so is the removal
  // — PLAY launches that copy and this is the one it offers to take away.
  const removal = useUninstallOffer(shown?.location.id ?? null, reload);
  // §24.3d offers Install where Play stands on a cloned project, so it is offered exactly where
  // there is no working copy to play.
  //
  // **§23.1's predicate, which is `primaryLocation !== null` and not a presence comparison.** The
  // card already reads it that way (`card/ProjectCard.tsx`), and the two disagree on a project
  // whose folder was moved: `projects.list` answers `presence: "missing"` with a non-null
  // `primaryLocation`, so a presence test here offered Install on the page while the tile offered
  // nothing for the same project at the same moment. One predicate, one owner.
  const hasWorkingCopy = detail !== null && detail.row.primaryLocation !== null;
  const install = useInstallOffer(detail === null || hasWorkingCopy ? null : detail.row.id);

  return (
    <div
      ref={rootRef}
      className={PROJECT_PAGE_ROOT_CLASS}
      data-testid="cp-page"
      data-gesture={racking === true ? 'rackout' : undefined}
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
              // [p3] §31.7: the `· <n> UNKNOWN` clause renders only on surfaces served by
              // `projects.get`, so the count comes from the detail and the hero invents none.
              unknownChecks={detail.completion?.unknown ?? null}
              onTogglePin={onTogglePin}
              // [p3] §33.2's input: §28's flat list, grouped by layer in the renderer (R120).
              // The opened hero is the one surface that receives it — never the grid, Peek, the
              // list, the quick-switch palette or triage.
              debt={detail.debt}
            />
            {/* [p2] §24.8's removal, mounted. The verdict is fetched when the affordance opens
                and at no other time; the page holds it because the rail is arrangement and the
                pre-flight belongs to the surface that knows which copy is shown. */}
            <Rail
              detail={detail}
              shown={shown}
              onChanged={reload}
              uninstallVerdict={removal.verdict}
              onOpenUninstall={removal.open}
              onUninstall={removal.remove}
              installPreview={install.preview}
              installRoots={install.roots}
              onChooseRoot={install.choose}
              onAddFolder={install.addFolder}
              onInstall={install.start}
              onOpenUpgrade={onOpenSettings}
            />
          </div>
          <div className="cp-col-right">
            <div className="cp-rise" style={{ animationDelay: cascadeDelay(0) }}>
              <Identity row={detail.row} visibility={detail.remote?.visibility ?? null} />
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
                  {/* §25.3: between the description and the NOTE block, for every project with
                      at least one location. §23 rules the zero-location case, where there is no
                      local copy to be the only copy of. */}
                  {/* [p3] §33.7's two clocks, in OVERVIEW rather than in the health tab. §30.7
                      mounts that tab on a predicate of its own; these are phase-1 derived facts
                      every project has whether or not a health reading exists, so mounting them
                      behind that predicate would hide a computed fact. This panel reads neither
                      the reading nor any exclusion — it is handed two condition values. */}
                  <ConditionPanel
                    signal={detail.row.conditionSignal}
                    material={detail.conditionMaterial}
                    isReference={detail.row.isReference}
                    isArchived={detail.row.isArchived}
                  />
                  <BackupStateBlock state={detail.backup} location={primary} now={deps.now()} />
                  <LocationsPanel
                    detail={detail}
                    shownId={shownId}
                    onShow={setShownId}
                    onChanged={reload}
                  />
                  <ReadmePanel
                    readme={detail.readme}
                    row={detail.row}
                    now={deps.now()}
                    locationId={shown?.location.id ?? null}
                    topics={detail.remote?.topics ?? []}
                  />
                  <NotePanel
                    projectId={detail.row.id}
                    row={detail.row}
                    note={detail.notes}
                    onChanged={reload}
                  />
                </>
              ) : null}
              {shownTab === 'activity' ? <ActivityTab detail={detail} /> : null}
              {shownTab === 'health' && settings !== null ? (
                <HealthTab
                  reading={detail.health}
                  settings={settings}
                  now={deps.now()}
                  onGrantSourceReading={() => {
                    // §29.8's grant, through the existing `settings.set`. §30 adds no command.
                    const grant = { contentScanEnabled: true };
                    void deps
                      .request('settings.set', {
                        // `SettingsPatch` carries every field and `null` means *leave it alone*,
                        // so a partial is widened by the one helper that owns that rule — a
                        // forgotten field written here would silently read as a change.
                        patch: toSettingsPatch(grant),
                      })
                      .then((answer) => {
                        setSettings(answer);
                        // The reading on this page and the shelf's rows were computed without
                        // the grant, so the stored write re-reads both, as the drawer's does.
                        if (movesHealthReading(grant)) onHealthInputsChanged?.();
                      })
                      .catch(() => undefined);
                  }}
                />
              ) : null}
              {/* [p3] §33.2: every item in text, whatever the layers could draw. Not behind the
                  settings read — the list needs no setting, and a failed read hides no item. */}
              {shownTab === 'health' ? <DebtList debt={detail.debt} /> : null}
              {/* [p3] §31.7: the ten ticks live here. §30.7 alone decides whether the tab
                  mounts; this renders nothing when the detail is NULL and takes no view on it. */}
              {shownTab === 'health' ? (
                <CompletionChecklist completion={detail.completion} />
              ) : null}
              {shownTab === 'remote' && detail.remote !== null ? (
                <RemoteTab
                  projectId={detail.row.id}
                  facts={detail.remote}
                  shown={shown}
                  now={deps.now()}
                  onOpenLink={(projectId, kind) => {
                    // The reply is a discriminated value the shell already acted on: `opened`,
                    // `declined`, `not_linkable` or `failed`. Nothing on this page changes on
                    // any of them, so it is awaited for its rejection and not for its answer.
                    void deps.openRemoteLink(projectId, kind).catch(() => undefined);
                  }}
                />
              ) : null}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
