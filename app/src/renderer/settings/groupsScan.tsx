/**
 * §11.3a groups 1–4.
 *
 * Two rulings this module applies. Group 1 draws **no root-removal control**: §11.3a's Rows
 * column draws a checkbox and a descend toggle, §17 forbids the destructive form, and disabling
 * a root is the reversible operation that already exists. And `roots: null` is *unknown*, not
 * *empty* — the caption reads `—`, never `0 OF 0 ACTIVE` (§7.7a).
 */
import type { ReactElement } from 'react';
import type { Root, RootId, RootState, TargetList, TargetRow } from '../../generated/protocol.js';
import { IPC_PICK_ROOT } from '../../shared/channels.js';
import { EXCLUSION_LIST } from '../../shared/skipList.js';
import { verifyNote } from '../project/rail/OpensIn.js';
import {
  SettingsGroup,
  SettingsRow,
  SwitchRow,
  type SettingsRowSpec,
  type SettingsSlots,
} from './rows.js';
import { SD } from './styles.js';

export const LAUNCH_TARGET_LANGUAGES = ['RS', 'TS', 'PY', 'C++', 'C#', 'ANY'] as const;

export const TARGET_FOOTNOTE =
  'A project can override its own target on its page. Whichever you pick, the launch is recorded — that is what playtime counts.';

/**
 * §11.3a's caption for group 3, which is **not** §10.1b's. The first-run screen says the list
 * is *editable in settings*; this is settings, no §2.4 command writes a user skip entry, and
 * `SettingsPatch` has no field for one — so that sentence would be false exactly where it is
 * read to decide. `EXCLUSION_LIST` itself is still the one shared constant.
 */
export const EXCLUSION_PRIVACY_CAPTION =
  'This list is the privacy policy. Nothing under these paths is read, indexed or counted.';

/** A language with no configured target. Absence, stated — never a target that reads as set. */
export const TARGET_UNSET = 'NOT SET';

/** §11.3a's four corrected statements, each matching the section that owns it word for word. */
export const SCANNING_STATEMENTS = [
  {
    label: 'The walk stops at every .git it finds',
    note: 'PER-ROOT ESCAPE HATCH FOR CLONES IN A SCRATCH REPO',
  },
  {
    label: 'An offline location freezes, it does not rot',
    note: 'ABSENT IS NOT ABANDONED · A TILE IS NEVER DELETED BY A SCAN',
  },
  {
    label: 'Source files are never read',
    note: 'NAMES AND TIMESTAMPS ONLY · PLUS README, LICENSE AND MANIFESTS AT THE ROOT, 256 KB EACH',
  },
  {
    label: 'Your scan roots, and the projects you opened most recently',
    note: 'DEPTH 1–2 OF EACH ROOT, PLUS THE ~30 MOST RECENT WORKTREES · NAMES AND TIMESTAMPS ONLY · EVERYTHING ELSE REFRESHES ON FOCUS OR ON DEMAND',
  },
] as const;

/** `null` is *not read*, and never renders as `0 OF 0 ACTIVE` (§7.7a). */
export function rootsCaption(roots: readonly Root[] | null): string {
  if (roots === null) return '—';
  return `${String(roots.filter((r) => r.enabled).length)} OF ${String(roots.length)} ACTIVE`;
}

export function rootStateLabel(state: RootState): 'WATCHED' | 'IGNORED' | 'OFFLINE' {
  return state === 'watched' ? 'WATCHED' : state === 'ignored' ? 'IGNORED' : 'OFFLINE';
}

/** `projectCount` is nullable on the wire, and an uncounted root prints no figure at all. */
export function rootProjectCountText(count: number | null): string | null {
  return count === null ? null : `${String(count)} PROJECTS`;
}

export const SCAN_GROUP_ROWS: readonly SettingsRowSpec[] = [
  {
    id: 'roots-enabled',
    group: 'roots',
    label: 'Scan this root',
    note: null,
    backing: { kind: 'command', command: 'roots.setEnabled' },
  },
  {
    id: 'roots-descend',
    group: 'roots',
    label: 'Descend into repositories',
    note: null,
    backing: { kind: 'command', command: 'roots.setDescend' },
  },
  {
    id: 'roots-add',
    group: 'roots',
    label: 'ADD A FOLDER',
    note: null,
    backing: { kind: 'shell', channel: IPC_PICK_ROOT },
  },
  {
    id: 'roots-rescan',
    group: 'roots',
    label: 'RESCAN NOW',
    note: null,
    backing: { kind: 'command', command: 'scan.start' },
  },
  ...LAUNCH_TARGET_LANGUAGES.map((tag) => ({
    id: `target-${tag}`,
    group: 'targets' as const,
    label: tag,
    note: null,
    backing: { kind: 'host' as const, slot: 'chooseLaunchTarget' as const },
  })),
  ...SCANNING_STATEMENTS.map((statement, index) => ({
    id: `scanning-${String(index)}`,
    group: 'scanning' as const,
    label: statement.label,
    note: statement.note,
    backing: { kind: 'statement' as const },
  })),
];

const spec = (id: string): SettingsRowSpec => {
  const found = SCAN_GROUP_ROWS.find((row) => row.id === id);
  if (found === undefined) throw new Error(`no settings row named ${id}`);
  return found;
};

export interface ScanGroupsProps {
  readonly roots: readonly Root[] | null;
  readonly targets: TargetList | null;
  readonly onSetEnabled: (id: RootId, enabled: boolean) => void;
  readonly onSetDescend: (id: RootId, descend: boolean) => void;
  readonly onAddFolder: () => void;
  readonly onRescan: () => void;
  readonly slots: SettingsSlots;
}

function targetNote(row: TargetRow | null, targets: TargetList | null): string | null {
  if (targets === null) return null; // the list was never read: unknown, not unset
  if (row === null) return TARGET_UNSET;
  return verifyNote(row.verifyState);
}

export function ScanGroups(props: ScanGroupsProps): ReactElement {
  const chooseLaunchTarget = props.slots.chooseLaunchTarget;

  return (
    <>
      <SettingsGroup id="roots" title="SCAN ROOTS" caption={rootsCaption(props.roots)}>
        {(props.roots ?? []).map((root) => (
          <div key={String(root.id)}>
            <SwitchRow
              spec={{
                ...spec('roots-enabled'),
                id: `roots-enabled-${String(root.id)}`,
                label: root.pathDisplay,
                note: `${root.provenance.toUpperCase()} · ${rootStateLabel(root.state)}`,
              }}
              checked={root.enabled}
              onChange={(next) => {
                props.onSetEnabled(root.id, next);
              }}
              {...(rootProjectCountText(root.projectCount) === null
                ? {}
                : { value: rootProjectCountText(root.projectCount) })}
            />
            <SwitchRow
              // Two roots would otherwise answer to one accessible name, and a voice command
              // naming it would toggle whichever the tree reached first.
              spec={{
                ...spec('roots-descend'),
                id: `roots-descend-${String(root.id)}`,
                label: `Descend into repositories under ${root.pathDisplay}`,
              }}
              checked={root.descendIntoRepos}
              onChange={(next) => {
                props.onSetDescend(root.id, next);
              }}
            />
          </div>
        ))}
        {/* Each action carries its own `data-row`, or the registry names a row the audit
            cannot find and a control could be added without anything checking it. */}
        <div style={SD.row}>
          <div style={SD.rowActions}>
            {/* §2.4: the path originates in a shell-owned native dialog. No field accepts one. */}
            <span data-row="roots-add">
              <button type="button" style={SD.buttonFilled} onClick={props.onAddFolder}>
                ADD A FOLDER
              </button>
            </span>
            <span data-row="roots-rescan">
              <button type="button" style={SD.buttonOutline} onClick={props.onRescan}>
                RESCAN NOW
              </button>
            </span>
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup id="targets" title="LAUNCH TARGETS" caption="PER LANGUAGE">
        {LAUNCH_TARGET_LANGUAGES.map((tag) => {
          const row = props.targets?.rows.find((r) => (r.language ?? 'ANY') === tag) ?? null;
          return (
            <SettingsRow
              key={tag}
              spec={{ ...spec(`target-${tag}`), note: targetNote(row, props.targets) }}
              {...(row === null ? {} : { value: row.execDisplay })}
              {...(chooseLaunchTarget === undefined
                ? {}
                : {
                    control: (
                      <button
                        type="button"
                        style={SD.buttonSmall}
                        onClick={() => {
                          chooseLaunchTarget(tag);
                        }}
                      >
                        CHANGE
                      </button>
                    ),
                  })}
            />
          );
        })}
        <p style={SD.rowNote}>{TARGET_FOOTNOTE}</p>
      </SettingsGroup>

      <SettingsGroup id="excluded" title="EXCLUDED FROM EVERY SCAN">
        <div style={{ ...SD.row, flexWrap: 'wrap', gap: '5px' }}>
          {EXCLUSION_LIST.map((entry) => (
            <span key={entry} style={SD.chip}>
              {entry}
            </span>
          ))}
        </div>
        <p style={SD.privacyCaption}>{EXCLUSION_PRIVACY_CAPTION}</p>
      </SettingsGroup>

      <SettingsGroup id="scanning" title="SCANNING · HOW IT WORKS">
        {SCANNING_STATEMENTS.map((statement, index) => (
          <SwitchRow key={statement.label} spec={spec(`scanning-${String(index)}`)} on />
        ))}
      </SettingsGroup>
    </>
  );
}
