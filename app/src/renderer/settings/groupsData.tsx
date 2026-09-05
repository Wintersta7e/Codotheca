/**
 * §11.3a groups 8, 8a and 9, the notification block and the footer.
 *
 * Three rulings this module applies.
 *  - **`FORGET A PROJECT` does not exist here in any spelling.** The row is `HIDE A PROJECT`,
 *    its action is `CHOOSE`, it sets `is_hidden`, and nothing is removed. It is drawn only when
 *    the host supplies the picker, because a `CHOOSE` button with nothing behind it is the
 *    dead-switch failure §11.3a names.
 *  - **Group 8a is omitted entirely without §1.4's card.** That is the rule's second limb.
 *  - **`SHOW REAL PATHS` is a real control** — it is the argument to the next `diag.bundle`, and
 *    the row says which state the last export was written in. §11.4 anonymises by default.
 */
import type { ReactElement } from 'react';
import type { DiagBundle, IdentityRow } from '../../generated/protocol.js';
import { IPC_REVEAL, type IndexLocation, type RevealTarget } from '../../shared/channels.js';
import { formatTrackedBytes } from '../format/size.js';
import {
  SettingsGroup,
  SettingsRow,
  SwitchRow,
  type SettingsRowSpec,
  type SettingsSlots,
} from './rows.js';
import { SD } from './styles.js';

export const EXPORT_NOTE = 'One JSON file · every project, session and note';
export const HIDE_NOTE = 'Keeps it off the shelf and out of every count. Nothing is removed.';
export const REAL_PATHS_NOTE = 'OFF EXPORTS BASENAMES AND VOLUME SHAPES ONLY';

/** §11.3a, verbatim. The control is cut; the line it justified stands. */
export const GITHUB_CONSEQUENCE =
  'Stars, issues, pull requests and CI state stay unknown until a token is added — and unknown is drawn as unknown, never as zero. Stored in the OS keychain, never in a config file.';

export const NOT_IN_THIS_BUILD = 'NOT IN THIS BUILD';

/** §11.3a states three that do not exist yet, and none of them can fire in phase 1. */
export const NOTIFICATIONS = [
  { label: 'A one-line summary on Sunday', note: 'SILENT ON AN EMPTY WEEK' },
  {
    label: 'A critical advisory in a project you have installed',
    note: 'NEEDS DEPENDENCY ADVISORIES',
  },
  { label: 'Your wrap is ready', note: 'NEEDS A WRAP' },
] as const;

export const FOOTER_LINES = [
  'CODOTHECA · GPL-3.0 · NO ACCOUNT · NO TELEMETRY',
  'THREE NOTIFICATIONS EXIST · NONE MENTIONS ABSENCE',
] as const;

/** `null` is *not read yet*, and is never rendered as `0 B`. */
export function indexSizeText(location: IndexLocation | null): string {
  return location === null
    ? '—'
    : `${location.pathDisplay} · ${formatTrackedBytes(location.sizeBytes)}`;
}

export function bundleResultText(bundle: DiagBundle | null): string | null {
  if (bundle === null) return null;
  const state = bundle.anonymised ? 'ANONYMISED' : 'REAL PATHS';
  return `${bundle.pathDisplay} · ${formatTrackedBytes(bundle.sizeBytes)} · ${state}`;
}

/** An unread identity list is unknown, and never `0 ADDRESSES ARE YOURS`. */
export function identityCaption(identities: readonly IdentityRow[] | null): string {
  if (identities === null) return '—';
  return `${String(identities.filter((row) => row.isUser).length)} ADDRESSES ARE YOURS`;
}

export const DATA_GROUP_ROWS: readonly SettingsRowSpec[] = [
  {
    id: 'data-index',
    group: 'data',
    label: 'INDEX AND LEDGER',
    note: null,
    backing: { kind: 'shell', channel: IPC_REVEAL },
  },
  {
    id: 'data-real-paths',
    group: 'data',
    label: 'SHOW REAL PATHS',
    note: REAL_PATHS_NOTE,
    backing: { kind: 'command', command: 'diag.bundle' },
  },
  {
    id: 'data-export',
    group: 'data',
    label: 'EXPORT EVERYTHING',
    note: EXPORT_NOTE,
    backing: { kind: 'command', command: 'diag.bundle' },
  },
  {
    id: 'data-hide',
    group: 'data',
    label: 'HIDE A PROJECT',
    note: HIDE_NOTE,
    backing: { kind: 'host', slot: 'chooseProjectToHide' },
  },
  {
    id: 'identity-add',
    group: 'identity',
    label: '+ ADD AN ADDRESS',
    note: null,
    backing: { kind: 'host', slot: 'addIdentityAddress' },
  },
  {
    id: 'github-statement',
    group: 'github',
    label: 'NOT CONNECTED',
    note: null,
    // [p2] §20.12: no longer a statement — the mounting surface supplies a panel whose every
    // row acts. `deadSwitch.test.tsx` walks this registry and the kind is what it reads.
    backing: { kind: 'host', slot: 'githubPanel' },
  },
  ...NOTIFICATIONS.map((notification, index) => ({
    id: `notification-${String(index)}`,
    group: 'notifications' as const,
    label: notification.label,
    note: `${notification.note} · ${NOT_IN_THIS_BUILD}`,
    backing: { kind: 'statement' as const },
  })),
];

const spec = (id: string): SettingsRowSpec => {
  const found = DATA_GROUP_ROWS.find((row) => row.id === id);
  if (found === undefined) throw new Error(`no settings row named ${id}`);
  return found;
};

export interface DataGroupsProps {
  readonly indexLocation: IndexLocation | null;
  readonly bundle: DiagBundle | null;
  readonly showRealPaths: boolean;
  readonly identities: readonly IdentityRow[] | null;
  readonly slots: SettingsSlots;
  readonly onReveal: (target: RevealTarget) => void;
  readonly onShowRealPaths: (next: boolean) => void;
  readonly onExport: () => void;
}

export function DataGroups(props: DataGroupsProps): ReactElement {
  const { chooseProjectToHide, identityCard, addIdentityAddress, githubPanel } = props.slots;
  const bundleText = bundleResultText(props.bundle);

  return (
    <>
      <SettingsGroup id="data" title="DATA">
        <SettingsRow
          spec={spec('data-index')}
          value={indexSizeText(props.indexLocation)}
          control={
            // §2.4: the renderer names one of the shell's two targets. It never sends a path.
            <button
              type="button"
              style={SD.buttonSmall}
              onClick={() => {
                props.onReveal('index');
              }}
            >
              REVEAL
            </button>
          }
        />
        <SwitchRow
          spec={spec('data-real-paths')}
          checked={props.showRealPaths}
          onChange={props.onShowRealPaths}
        />
        <SettingsRow
          spec={spec('data-export')}
          {...(bundleText === null ? {} : { value: bundleText })}
          control={
            <button type="button" style={SD.buttonSmall} onClick={props.onExport}>
              EXPORT
            </button>
          }
        />
        {/* §11.3a: FORGET A PROJECT is replaced, not renamed, and is drawn with a picker or
            not at all — a walk re-creates a project the index was told to drop, so the cut
            control is the one that appeared to work and silently reverted. */}
        {chooseProjectToHide !== undefined && (
          <SettingsRow
            spec={spec('data-hide')}
            control={
              <button type="button" style={SD.buttonSmall} onClick={chooseProjectToHide}>
                CHOOSE
              </button>
            }
          />
        )}
      </SettingsGroup>

      {/* §11.3a's second limb: without §1.4's card there is nothing to re-open, so nothing
          is drawn — not a group header over an empty box. */}
      {identityCard !== undefined && (
        <SettingsGroup id="identity" title="IDENTITY" caption={identityCaption(props.identities)}>
          {identityCard()}
          {addIdentityAddress !== undefined && (
            <div style={SD.row} data-row="identity-add">
              <button type="button" style={SD.buttonDashed} onClick={addIdentityAddress}>
                + ADD AN ADDRESS
              </button>
            </div>
          )}
        </SettingsGroup>
      )}

      <SettingsGroup id="github" title="GITHUB">
        {/* [p2] §20.12: a real control, supplied by the mounting surface. `--absent`, never the
            accent: amber says *something is wrong here*, and the absence of a token is a
            shipped state rather than a defect. Without the slot the statement stands, which is
            §11.3a's second limb. */}
        {/* The registry's row id stays on the wrapper in **both** shapes, so `deadSwitch`'s
            walk finds the row it declares whichever way the group is drawn. The id predates the
            control and names the row, not its contents. */}
        <div data-row="github-statement">
          {githubPanel === undefined ? (
            <div style={SD.blockAbsent}>
              <span style={SD.rowLabel}>NOT CONNECTED</span>
              <p style={SD.rowNote}>{GITHUB_CONSEQUENCE}</p>
            </div>
          ) : (
            githubPanel()
          )}
        </div>
      </SettingsGroup>

      <SettingsGroup id="notifications" title="NOTIFICATIONS">
        <div style={SD.blockAbsent}>
          {NOTIFICATIONS.map((notification, index) => (
            <SwitchRow
              key={notification.label}
              spec={spec(`notification-${String(index)}`)}
              on={false}
            />
          ))}
        </div>
        <footer style={SD.footer}>
          {FOOTER_LINES.map((line) => (
            <span key={line}>{line}</span>
          ))}
        </footer>
      </SettingsGroup>
    </>
  );
}
