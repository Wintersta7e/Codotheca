/**
 * §8.5.1's rail. `GITHUB`, `RESTORE ↻`, `CLONE` and `REFERENCE ONLY` are cut by that table and
 * are not rendered in any state — a reference project gets `PLAY` like everything else.
 *
 * The reroll offset lives here rather than on the page: its only consumer is the stepper's own
 * readout, and the hero re-paints from `art_ready`, which `useProjectDetail` already follows.
 */
import { useEffect, useRef, useState, type ReactElement } from 'react';

import type {
  InstallPreview,
  LocationDetail,
  ProjectDetail,
  TargetRow,
  UninstallVerdict,
} from '../../../generated/protocol';
import { InstallControl } from '../../install/InstallControl';
import { formatPlaytime } from '../../format/playtime';
import { useProjectPageDeps } from '../deps';
import { UninstallControl } from '../uninstall/UninstallControl';
import { UNINSTALL_OPEN_LABEL, UNINSTALL_OPEN_NOTE } from '../uninstall/uninstallCopy';
import { editorTargets, OpensIn } from './OpensIn';
import { RerollStepper } from './RerollStepper';
import { ctaState, type CtaState } from './statFormat';

/**
 * Copy this plan adds. §8.5.1's CTA table has no row for it, and §11.3a forbids the alternative:
 * `CHOOSE AN APP` with no `OPENS IN` to reach is a control that cannot act.
 */
export const NO_APP_STATEMENT = 'NO APP DETECTED';

/** The rail's own terminal button; the lowest sort index wins, as everywhere else targets rank. */
export function terminalTarget(targets: readonly TargetRow[]): TargetRow | null {
  const terminals = targets.filter((t) => t.kind === 'terminal');
  const sorted = [...terminals].sort((a, b) => a.sortIndex - b.sortIndex);
  return sorted[0] ?? null;
}

export interface RailProps {
  detail: ProjectDetail;
  /**
   * [p2] §24.3d's preview, or `undefined` when this rail is not offering Install. The rail is the
   * second of exactly two mount points in the renderer.
   */
  installPreview?: InstallPreview | null;
  onInstall?: () => void;
  onOpenUpgrade?: () => void;
  /**
   * [p2-24b] §24.6's verdict, or `undefined` when this rail is not offering Uninstall. `null` is
   * *the pre-flight is in flight* — a distinct state the control renders as checking, never as an
   * enabled button it later takes away.
   */
  uninstallVerdict?: UninstallVerdict | null | undefined;
  onUninstall?: () => void;
  /**
   * [p2] §24.8: the affordance that opens the check. Its absence is what means *this rail is not
   * offering Uninstall at all* — `uninstallVerdict` alone could not say that and be closed, since
   * `undefined` there is also the state before the first press. The pre-flight fetches from the
   * remote, so nothing but this press may start one.
   */
  onOpenUninstall?: () => void;
  shown: LocationDetail | null;
  onChanged: () => void;
}

export function Rail({
  detail,
  shown,
  onChanged,
  installPreview,
  onInstall,
  onOpenUpgrade,
  uninstallVerdict,
  onUninstall,
  onOpenUninstall,
}: RailProps): ReactElement {
  const deps = useProjectPageDeps();
  const [offset, setOffset] = useState(detail.rerollOffset);
  const [launching, setLaunching] = useState(false);
  const opensInRef = useRef<HTMLButtonElement>(null);
  // §2.4: launch is non-idempotent, so the guard has to hold within one tick as well as across
  // a render. State alone re-opens the window for as long as the re-render takes.
  const inFlight = useRef(false);

  useEffect(() => {
    setOffset(detail.rerollOffset);
  }, [detail.rerollOffset]);

  const editors = editorTargets(detail.targets);
  const terminal = terminalTarget(detail.targets);
  const hasPresentLocation = detail.locations.some((l) => l.presence === 'present');
  const cta = ctaState({
    hasLiveSession: detail.liveSession !== null,
    isArchived: detail.row.isArchived,
    hasPresentLocation,
    hasResolvedTarget: detail.resolvedTarget !== null,
  });

  const launch = (targetId: TargetRow['id']): void => {
    if (inFlight.current || shown === null) return;
    inFlight.current = true;
    setLaunching(true);
    deps
      .request('projects.launch', {
        projectId: detail.row.id,
        locationId: shown.location.id,
        targetId,
      })
      .catch(() => {
        // §2.4: launch is non-idempotent and is never replayed. The failure window owns the copy.
      })
      .finally(() => {
        inFlight.current = false;
        setLaunching(false);
      });
  };

  const onCta = (): void => {
    if (cta.kind !== 'action') return;
    if (cta.word === 'CHOOSE AN APP') {
      opensInRef.current?.focus();
      return;
    }
    const target = detail.resolvedTarget;
    if (target !== null) launch(target.target.id);
  };

  const ctaIsDead = cta.kind === 'action' && cta.word === 'CHOOSE AN APP' && editors.length === 0;
  // The dead `CHOOSE AN APP` becomes a statement of the same shape rather than a fourth branch,
  // so the render below is total over `CtaState` and has no unreachable arm to get wrong.
  const control: CtaState = ctaIsDead ? { kind: 'statement', text: NO_APP_STATEMENT } : cta;

  return (
    <div className="cp-rail" data-testid="cp-rail">
      {/* [p2] §24.3d: Install occupies the slot Play occupies on a cloned project, so it stands
          above the CTA rather than beside it. The second of exactly two mount points in the
          renderer; `app/test/installSites.test.ts` fails on a third. */}
      {installPreview === undefined ? null : (
        <InstallControl
          preview={installPreview}
          onInstall={onInstall ?? (() => {})}
          onOpenUpgrade={onOpenUpgrade ?? (() => {})}
        />
      )}
      {control.kind === 'statement' ? (
        <div className="cp-cta-statement" data-testid="cp-cta-statement">
          {control.text}
        </div>
      ) : (
        <button
          type="button"
          className={control.word === 'REOPEN' ? 'cp-cta cp-cta-reopen' : 'cp-cta'}
          data-testid="cp-cta"
          aria-busy={launching}
          onClick={onCta}
        >
          {control.word}
        </button>
      )}

      <OpensIn
        ref={opensInRef}
        projectId={detail.row.id}
        targets={detail.targets}
        resolved={detail.resolvedTarget}
        onChanged={onChanged}
      />

      {terminal === null ? null : (
        <button
          type="button"
          className="cp-terminal"
          data-testid="cp-terminal"
          onClick={() => {
            launch(terminal.id);
          }}
        >
          TERMINAL
        </button>
      )}

      <RerollStepper
        projectId={detail.row.id}
        seedBasename={detail.seedBasename}
        offset={offset}
        onOffset={setOffset}
      />

      <div className="cp-stats">
        <div className="cp-stat">
          <div className="cp-stat-label">PLAYTIME</div>
          <div className="cp-stat-value" data-testid="cp-stat-playtime">
            {formatPlaytime(detail.playtimeSeconds)}
          </div>
        </div>
        <div className="cp-stat">
          <div className="cp-stat-label">BIRTH</div>
          <div className="cp-stat-value" data-testid="cp-stat-birth">
            {detail.row.birthYear === null ? 'NOT KNOWN' : String(detail.row.birthYear)}
          </div>
        </div>
      </div>

      {/* [p2-24b] §24.5: Uninstall's **one** mount point, and it stands below the stats rather
          than beside Play — there is nothing to gain from putting a removal where the eye lands
          first. A project with no present location has no working copy to remove, so the slot is
          not merely disabled there; it is absent.

          [p2] §24.8: before the first press there is no verdict and there must be no fetch, so
          the slot holds the affordance that opens the check. The control replaces it from the
          press onwards — `null` is checking, and it never renders enabled then takes itself
          away. */}
      {onOpenUninstall === undefined || !hasPresentLocation ? null : uninstallVerdict ===
        undefined ? (
        <div className="cp-uninstall" data-state="closed">
          <p className="cp-uninstall__note">{UNINSTALL_OPEN_NOTE}</p>
          <button
            type="button"
            className="cp-uninstall__open"
            data-testid="cp-uninstall-open"
            onClick={onOpenUninstall}
          >
            {UNINSTALL_OPEN_LABEL}
          </button>
        </div>
      ) : (
        <UninstallControl verdict={uninstallVerdict} onUninstall={onUninstall ?? (() => {})} />
      )}
    </div>
  );
}
