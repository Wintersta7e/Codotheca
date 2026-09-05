/**
 * §11's three overlays and the palette, over whichever top-level surface is mounted.
 *
 * They are **not** routes: closing the drawer must not navigate, and the scan summary opens
 * over the shelf and the project page alike. `route.ts` decides what is behind them; this
 * decides only what is on top.
 */
import { useCallback, useEffect, useMemo, useRef, type ReactElement } from 'react';

import type { LocationId, Problems, ProjectId, ProjectRow } from '../../generated/protocol.js';
import type { FailureFact } from '../failure/copy.js';
import { FailureWindow } from '../failure/FailureWindow.js';
import type { ResolvedTier } from '../motion/tier.js';
import { QuickSwitchHost } from '../palette/QuickSwitchHost.js';
import { seedOf, appearanceFor, fadeFor } from '../art/appearance.js';
import { SettingsDrawer } from '../settings/Drawer.js';
import type { SettingsSlots } from '../settings/rows.js';
import { GithubPanelHost } from '../settings/GithubPanelHost.js';
import { ScanSummary } from '../summary/ScanSummary.js';
import type { AppDeps } from './deps.js';

export interface SurfaceHostProps {
  readonly deps: AppDeps;
  readonly tier: ResolvedTier;
  /** Non-null exactly when `route.kind === 'failure'`. */
  readonly failure: FailureFact | null;
  readonly settingsOpen: boolean;
  readonly onCloseSettings: () => void;
  readonly summaryOpen: boolean;
  readonly onCloseSummary: () => void;
  readonly problems: Problems | null;
  readonly onProblemsChanged: () => void;
  readonly rows: readonly ProjectRow[];
  readonly liveSessionProjectIds: ReadonlySet<ProjectId>;
  readonly onOpenProject: (id: ProjectId) => void;
  readonly onLaunch: (projectId: ProjectId, locationId: LocationId) => void;
  /**
   * §8.0a's `SWITCH` control opens the palette too, and it is above this host rather than in it.
   * A counter rather than a boolean: two presses in a row are two openings, and a flag that was
   * already true would make the second one silent.
   */
  readonly openPaletteNonce?: number;
}

export function SurfaceHost(props: SurfaceHostProps): ReactElement {
  const { deps, tier, failure, rows } = props;
  const { request, reveal, indexLocation, pickRoot, onShortcutState } = deps;
  /**
   * §11.3a's dead-switch rule, applied by omission. A slot is filled only where something
   * answers it: `identityCard` needs a host that runs `identity.confirm` and writes §8.0's
   * dismissal, `chooseProjectToHide` and `chooseLaunchTarget` need a picker neither §11.3 nor
   * any landed plan builds. An entry filled with a no-op would draw a row that does nothing,
   * which §11.3a forbids by name — so the absent ones stay absent and the drawer draws no row.
   */
  const slots = useMemo<SettingsSlots>(
    () => ({
      // [p2] §20.12: filled, because something now answers it — all eight `accounts.*` commands
      // reach a module in the core. `onConnectPat` is deliberately not supplied: the PAT path
      // needs a host field no landed plan draws, so its button stays undrawn rather than
      // becoming a control that does nothing.
      githubPanel: () => <GithubPanelHost request={request} subscribe={deps.subscribe} />,
    }),
    [request, deps.subscribe],
  );

  // One registration on the shell channel, and the same callback reused for the top bar's own
  // control — a second subscription would be a second palette listening to half the openings.
  const openPalette = useRef<(() => void) | null>(null);
  const subscribeShellOpen = useCallback(
    (cb: () => void) => {
      openPalette.current = cb;
      return deps.onOpenPalette(cb);
    },
    [deps],
  );
  const nonce = props.openPaletteNonce ?? 0;
  useEffect(() => {
    if (nonce > 0) openPalette.current?.();
  }, [nonce]);

  return (
    <>
      <SettingsDrawer
        open={props.settingsOpen}
        onClose={props.onCloseSettings}
        deps={{
          call: request,
          shell: {
            pickRoot,
            reveal,
            indexLocation,
            // The drawer's own contract takes the bridge's disposer-less shape; the fan-out's
            // disposer is dropped here because the drawer holds it for the window's life.
            onShortcutState: (cb) => {
              onShortcutState(cb);
            },
          },
          tier,
        }}
        slots={slots}
      />
      {props.summaryOpen && props.problems !== null && (
        <ScanSummary
          problems={props.problems}
          request={request}
          onOpenProject={(id) => {
            props.onCloseSummary();
            props.onOpenProject(id);
          }}
          onChanged={props.onProblemsChanged}
        />
      )}
      {failure !== null && (
        <FailureWindow
          fact={failure}
          logPath={deps.logPath}
          tier={tier}
          nowMs={deps.nowMs}
          /*
           * §11.2a's QUIT. Closing the window ends the process — `window-all-closed` quits —
           * and the renderer needs no privileged verb for it.
           *
           * `corrupt_index`'s primary reads REBUILD and does the same thing, which is the one
           * label here that is not literally true: no command in the schema's forty-two runs a
           * rebuild, nothing calls `Index::rebuild`, and the renderer cannot relaunch. The
           * quarantine has already happened by the time this window is drawn, so the next
           * launch does rebuild — but this button does not. Recorded rather than dressed up.
           */
          onPrimary={() => {
            window.close();
          }}
          onSecondary={() => {
            window.close();
          }}
        />
      )}
      <QuickSwitchHost
        rows={rows}
        liveSessionProjectIds={props.liveSessionProjectIds}
        effectsTier={tier}
        jewelFor={(row) => appearanceFor(seedOf(row), fadeFor(row), row.primaryLanguage).jewel}
        now={deps.now}
        onLaunch={props.onLaunch}
        onOpenPage={props.onOpenProject}
        subscribeShellOpen={subscribeShellOpen}
        ambientContext="shelf"
      />
    </>
  );
}
