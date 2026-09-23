/**
 * §11.3a's drawer: a 400px panel at the right edge of a scrim. Esc closes; the backdrop is the
 * only close-on-click target; clicks inside the panel do not bubble to it.
 *
 * **The drawer holds no switch position of its own.** `settings.get` is the only source, and
 * `settings.set` returns the whole `Settings` back, so the drawer replaces its snapshot with
 * the core's answer rather than optimistically flipping. A rejected or clamped write therefore
 * cannot leave a switch showing a value the database does not hold — which is the dead-switch
 * rule in its temporal form: a switch that *appears* to have worked is worse than one that
 * visibly did not.
 */
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactElement,
} from 'react';
import type {
  DiagBundle,
  IdentityRow,
  Root,
  RootId,
  Settings,
  SettingsPatch,
  TargetList,
} from '../../generated/protocol.js';
import type {
  BridgeReply,
  IndexLocation,
  PickRootReply,
  RevealTarget,
  ShortcutState,
} from '../../shared/channels.js';
import { resolveKey } from '../keyboard/contexts.js';
import type { ResolvedTier } from '../motion/tier.js';
import { unwrapReply, type CoreCall } from '../core/call.js';
import { DATA_GROUP_ROWS, DataGroups } from './groupsData.js';
import { HEALTH_GROUP_ROWS, HealthGroups } from './groupsHealth.js';
import { MOTION_GROUP_ROWS, MotionGroups } from './groupsMotion.js';
import { SCAN_GROUP_ROWS, ScanGroups } from './groupsScan.js';
import { SETTINGS_GROUP_ORDER, type SettingsRowSpec, type SettingsSlots } from './rows.js';
import { SD, backdropAnimation, panelAnimation } from './styles.js';

export const ESC_CHIP_LABEL = 'ESC';
export const DRAWER_TITLE = 'SETTINGS';

/** The registry every dead-switch audit walks. One entry per drawn row, in §11.3a's order. */
export const SETTINGS_ROWS: readonly SettingsRowSpec[] = [
  ...SCAN_GROUP_ROWS,
  ...HEALTH_GROUP_ROWS,
  ...MOTION_GROUP_ROWS,
  ...DATA_GROUP_ROWS,
].sort((a, b) => SETTINGS_GROUP_ORDER.indexOf(a.group) - SETTINGS_GROUP_ORDER.indexOf(b.group));

/**
 * `SettingsPatch` carries every field, and `null` means *leave it alone* — so a partial has to
 * be widened here rather than at each call site, where a forgotten field would silently read as
 * a change. `false` is a value and survives; only `undefined` becomes `null`.
 */
export function toSettingsPatch(patch: Partial<Settings>): SettingsPatch {
  return {
    effectsTier: patch.effectsTier ?? null,
    reducedMotionOverride: patch.reducedMotionOverride ?? null,
    autostart: patch.autostart ?? null,
    residentShortcut: patch.residentShortcut ?? null,
    roastEnabled: patch.roastEnabled ?? null,
    logLevel: patch.logLevel ?? null,
    installRootId: patch.installRootId ?? null,
    contentScanEnabled: patch.contentScanEnabled ?? null,
    healthChecks: patch.healthChecks ?? null,
  };
}

/**
 * [p3] Whether a write moves any project's health reading: a check's switch takes its items off
 * every project (§30.9), and the source-reading grant decides whether `todo_marker` is `off`
 * (R142). No other field reaches a reading, and a write of one re-reads nothing.
 */
export function movesHealthReading(patch: Partial<Settings>): boolean {
  return patch.healthChecks !== undefined || patch.contentScanEnabled !== undefined;
}

/**
 * The four shell capabilities the drawer needs, in the shapes `CodothecaBridge` already has —
 * so the mount point passes `window.codotheca` and wraps nothing. `Drawer.test.tsx` assigns a
 * bridge to this type, which is what stops the two drifting apart.
 *
 * `pickRoot` carries a flag, never a path: the renderer asks, the shell opens the dialog, and
 * the chosen folder never round-trips through the sandbox (§2.4).
 */
export interface SettingsShell {
  readonly pickRoot: (confirmLarge: boolean) => Promise<PickRootReply>;
  readonly reveal: (target: RevealTarget) => Promise<unknown>;
  readonly indexLocation: () => Promise<unknown>;
  readonly onShortcutState: (cb: (state: ShortcutState) => void) => void;
}

export interface SettingsDrawerDeps {
  readonly call: CoreCall;
  readonly shell: SettingsShell;
  readonly tier: ResolvedTier;
}

export interface SettingsDrawerProps {
  readonly open: boolean;
  readonly onClose: () => void;
  readonly deps: SettingsDrawerDeps;
  readonly slots: SettingsSlots;
  /**
   * Called once the core has stored a write that `movesHealthReading`. `settings.set` raises no
   * event, so whoever holds a reading — the shelf's rows, an open page's detail — hears it here.
   */
  readonly onHealthInputsChanged?: (() => void) | undefined;
  /**
   * Called with every answer to a write. The motion rows act on the window rather than on the
   * core, and `settings.set` raises no event, so the window hears a stored tier here.
   */
  readonly onSettings?: ((settings: Settings) => void) | undefined;
}

export function SettingsDrawer(props: SettingsDrawerProps): ReactElement | null {
  const { call, shell, tier } = props.deps;
  const [settings, setSettings] = useState<Settings | null>(null);
  const [roots, setRoots] = useState<readonly Root[] | null>(null);
  const [targets, setTargets] = useState<TargetList | null>(null);
  const [indexLocation, setIndexLocation] = useState<IndexLocation | null>(null);
  const [identities, setIdentities] = useState<readonly IdentityRow[] | null>(null);
  const [bundle, setBundle] = useState<DiagBundle | null>(null);
  const [showRealPaths, setShowRealPaths] = useState(false);
  const [shortcut, setShortcut] = useState<ShortcutState>({ chord: null, registered: false });
  const [recording, setRecording] = useState(false);
  const panel = useRef<HTMLDivElement | null>(null);

  /**
   * A read that fails leaves its state `null`, which every group renders as *unknown*. Folding
   * a refusal into an empty list would teach a configured machine as an unconfigured one.
   */
  const readRoots = useCallback(() => {
    call('roots.list', {}).then(
      (rows) => {
        setRoots(rows);
      },
      () => undefined,
    );
  }, [call]);

  useEffect(() => {
    if (!props.open) return;
    call('settings.get', {}).then(setSettings, () => undefined);
    call('targets.list', { projectId: null, locationId: null }).then(setTargets, () => undefined);
    call('identity.list', {}).then(setIdentities, () => undefined);
    readRoots();
    shell.indexLocation().then(
      (reply) => {
        setIndexLocation(unwrapReply<IndexLocation>(reply as BridgeReply));
      },
      () => undefined,
    );
    shell.onShortcutState(setShortcut);
    panel.current?.focus();
  }, [props.open, call, shell, readRoots]);

  /**
   * One writer: the core answers with the whole `Settings`, and that answer is the state. A
   * refused write moved nothing, so only an answered one tells the readings to re-read.
   */
  const { onHealthInputsChanged, onSettings } = props;
  const patch = useCallback(
    (next: Partial<Settings>) => {
      call('settings.set', { patch: toSettingsPatch(next) }).then(
        (answer) => {
          setSettings(answer);
          onSettings?.(answer);
          if (movesHealthReading(next)) onHealthInputsChanged?.();
        },
        () => undefined,
      );
    },
    [call, onHealthInputsChanged, onSettings],
  );

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    const target = event.target as { tagName?: string; isContentEditable?: boolean } | null;
    const resolved = resolveKey('settings', {
      key: event.key,
      code: event.code,
      altKey: event.altKey,
      ctrlKey: event.ctrlKey,
      shiftKey: event.shiftKey,
      metaKey: event.metaKey,
      target,
    });
    if (resolved?.action !== 'settings.close') return;
    if (resolved.preventDefault) event.preventDefault();
    props.onClose();
  };

  if (!props.open) return null;

  return (
    <div
      data-testid="sd-backdrop"
      style={{ ...SD.backdrop, ...backdropAnimation(tier) }}
      onClick={props.onClose}
    >
      {/* The panel owns the settings key context (§11.7) and swallows the backdrop's
          close-on-click, so both handlers sit on it rather than on the scrim. */}
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={DRAWER_TITLE}
        tabIndex={-1}
        style={{ ...SD.panel, ...panelAnimation(tier) }}
        onClick={(event) => {
          event.stopPropagation();
        }}
        onKeyDown={onKeyDown}
      >
        <header style={SD.header}>
          <span style={SD.title}>{DRAWER_TITLE}</span>
          <span style={SD.escChip}>{ESC_CHIP_LABEL}</span>
        </header>
        <div style={SD.body}>
          <ScanGroups
            roots={roots}
            targets={targets}
            onSetEnabled={(id: RootId, enabled) => {
              call('roots.setEnabled', { id, enabled }).then(readRoots, () => undefined);
            }}
            onSetDescend={(id: RootId, descendIntoRepos) => {
              call('roots.setDescend', { id, descendIntoRepos }).then(readRoots, () => undefined);
            }}
            onAddFolder={() => {
              // `false`: nothing here has shown the user a large-directory estimate to accept.
              shell.pickRoot(false).then(
                (reply) => {
                  if (reply.kind === 'added') readRoots();
                },
                () => undefined,
              );
            }}
            onRescan={() => {
              call('scan.start', { full: false }).then(
                () => undefined,
                () => undefined,
              );
            }}
            contentScanEnabled={settings === null ? null : settings.contentScanEnabled}
            onSetContentScan={(contentScanEnabled) => {
              patch({ contentScanEnabled });
            }}
            slots={props.slots}
          />
          {settings !== null && <HealthGroups settings={settings} onPatch={patch} />}
          {settings !== null && (
            <MotionGroups
              settings={settings}
              shortcut={shortcut}
              recording={recording}
              onPatch={patch}
              onRecordChord={() => {
                setRecording(true);
              }}
              onChordCaptured={(chord) => {
                setRecording(false);
                // A cancelled recording binds nothing and clears nothing.
                if (chord !== null) patch({ residentShortcut: chord });
              }}
            />
          )}
          <DataGroups
            indexLocation={indexLocation}
            bundle={bundle}
            showRealPaths={showRealPaths}
            identities={identities}
            slots={props.slots}
            onReveal={(target) => {
              shell.reveal(target).then(
                () => undefined,
                () => undefined,
              );
            }}
            onShowRealPaths={setShowRealPaths}
            onExport={() => {
              call('diag.bundle', { includeRealPaths: showRealPaths }).then(
                setBundle,
                () => undefined,
              );
            }}
          />
        </div>
      </div>
    </div>
  );
}
