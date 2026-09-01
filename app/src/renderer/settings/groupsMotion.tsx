/**
 * §11.3a groups 5–7.
 *
 * **One switch per behaviour.** §11.6's scheduled flicker rides the tier and gets no row; the
 * ambient key-light drift is out of phase 1 entirely; decay rendering does not exist; level, XP
 * and quests are §0. Group 5 draws the four-way tier, the reduced-motion override and one
 * density statement, and nothing else. Density is `view_state` (§1.9) and its control is the top
 * bar's (§8.0a), so the drawer names its home and carries no second copy of it.
 */
import { useState, type KeyboardEvent, type ReactElement } from 'react';
import type { Settings } from '../../generated/protocol.js';
import type { ShortcutState } from '../../shared/channels.js';
import { EFFECTS_TIERS } from '../../shared/effectsTier.js';
import { residentShortcutStatusText } from '../../shared/chord.js';
import { CHORD_PROMPT, bindOutcomeFor, captureChord } from './chord.js';
import { SettingsGroup, SettingsRow, SwitchRow, type SettingsRowSpec } from './rows.js';
import { SD } from './styles.js';

export const EFFECTS_TIER_LABELS = ['AUTO', 'FULL', 'REDUCED', 'OFF'] as const;
export const EFFECTS_NOTE = 'FULL · REDUCED AND OFF EACH KEEP A STATIC EQUIVALENT';
export const REDUCED_OVERRIDE_NOTE = 'CLAMPS THE TIER TO AT MOST REDUCED';
export const DENSITY_STATEMENT_NOTE = 'SET IN THE TOP BAR · COMPACT · DEFAULT · LARGE';
export const RESIDENCY_STATEMENT_NOTE =
  '9 MS TO SHOW WHILE THE WINDOW LIVES · 134 MS AFTER IT IS DESTROYED';
export const AUTOSTART_NOTE =
  '307 MB EMPTY · 522 MB WITH A FULL SHELF · 232 MB WITH THE WINDOW DESTROYED';
export const ROAST_NOTE = 'NEVER ON THE SHELF · NEVER DURING TRIAGE';
export const SHORTCUT_LABEL = 'Show Codotheca from anywhere';
export const REBIND_LABEL = 'REBIND';

export const MOTION_GROUP_ROWS: readonly SettingsRowSpec[] = [
  {
    id: 'motion-tier',
    group: 'motion',
    label: 'Effects',
    note: EFFECTS_NOTE,
    backing: { kind: 'command', command: 'settings.set' },
  },
  {
    id: 'motion-reduced',
    group: 'motion',
    label: 'Respect the system reduced-motion setting',
    note: REDUCED_OVERRIDE_NOTE,
    backing: { kind: 'command', command: 'settings.set' },
  },
  {
    id: 'motion-density',
    group: 'motion',
    label: 'Card size is a property of the view, not of the app',
    note: DENSITY_STATEMENT_NOTE,
    backing: { kind: 'statement' },
  },
  {
    id: 'residency-hybrid',
    group: 'residency',
    label: 'The window is destroyed 30 minutes after last use',
    note: RESIDENCY_STATEMENT_NOTE,
    backing: { kind: 'statement' },
  },
  {
    id: 'residency-autostart',
    group: 'residency',
    label: 'Start with the system',
    note: AUTOSTART_NOTE,
    backing: { kind: 'command', command: 'settings.set' },
  },
  {
    id: 'residency-chord',
    group: 'residency',
    label: SHORTCUT_LABEL,
    note: null,
    backing: { kind: 'command', command: 'settings.set' },
  },
  {
    id: 'projectpage-roast',
    group: 'projectPage',
    label: 'Dry one-line notes on an opened project',
    note: ROAST_NOTE,
    backing: { kind: 'command', command: 'settings.set' },
  },
];

const spec = (id: string): SettingsRowSpec => {
  const found = MOTION_GROUP_ROWS.find((row) => row.id === id);
  if (found === undefined) throw new Error(`no settings row named ${id}`);
  return found;
};

/** The override clamps rather than replaces, and the row says so instead of quietly winning. */
export function effectiveTierNote(settings: Settings): string | null {
  if (!settings.reducedMotionOverride) return null;
  if (settings.effectsTier === 'reduced' || settings.effectsTier === 'off') return null;
  return `${settings.effectsTier.toUpperCase()} IS CLAMPED TO REDUCED WHILE THIS IS ON`;
}

export interface MotionGroupsProps {
  readonly settings: Settings;
  readonly shortcut: ShortcutState;
  readonly recording: boolean;
  readonly onPatch: (patch: Partial<Settings>) => void;
  readonly onRecordChord: () => void;
  /** `null` is a cancelled recording, which binds nothing and clears nothing. */
  readonly onChordCaptured: (chord: string | null) => void;
}

export function MotionGroups(props: MotionGroupsProps): ReactElement {
  const [warning, setWarning] = useState<string | null>(null);
  const status = residentShortcutStatusText(bindOutcomeFor(props.shortcut));

  const onRecorderKey = (event: KeyboardEvent<HTMLButtonElement>): void => {
    if (!props.recording) return;
    // The drawer closes on Escape (§11.3a) and the recorder is inside it, so a key that means
    // *cancel the recording* must not also mean *close the drawer*.
    event.preventDefault();
    event.stopPropagation();
    const capture = captureChord(event);
    if (capture.kind === 'incomplete') return;
    if (capture.kind === 'cancelled') {
      setWarning(null);
      props.onChordCaptured(null);
      return;
    }
    setWarning(capture.warning);
    props.onChordCaptured(capture.chord);
  };

  return (
    <>
      <SettingsGroup id="motion" title="MOTION">
        <SettingsRow
          spec={spec('motion-tier')}
          control={
            <div role="radiogroup" aria-label="Effects" style={SD.rowActions}>
              {EFFECTS_TIERS.map((tier, index) => {
                const current = props.settings.effectsTier === tier;
                return (
                  <button
                    key={tier}
                    type="button"
                    role="radio"
                    aria-checked={current}
                    aria-label={EFFECTS_TIER_LABELS[index]}
                    style={current ? SD.buttonFilled : SD.buttonSmall}
                    onClick={() => {
                      props.onPatch({ effectsTier: tier });
                    }}
                  >
                    {EFFECTS_TIER_LABELS[index]}
                  </button>
                );
              })}
            </div>
          }
        />
        <SwitchRow
          spec={{
            ...spec('motion-reduced'),
            note: effectiveTierNote(props.settings) ?? REDUCED_OVERRIDE_NOTE,
          }}
          checked={props.settings.reducedMotionOverride}
          onChange={(next) => {
            props.onPatch({ reducedMotionOverride: next });
          }}
        />
        <SwitchRow spec={spec('motion-density')} on />
      </SettingsGroup>

      <SettingsGroup id="residency" title="RESIDENCY">
        <SwitchRow spec={spec('residency-hybrid')} on />
        <SwitchRow
          spec={spec('residency-autostart')}
          checked={props.settings.autostart}
          onChange={(next) => {
            props.onPatch({ autostart: next });
          }}
        />
        <SettingsRow
          spec={{ ...spec('residency-chord'), note: warning }}
          value={props.recording ? CHORD_PROMPT : status}
          control={
            <button
              type="button"
              style={SD.buttonSmall}
              aria-label={props.recording ? CHORD_PROMPT : `${SHORTCUT_LABEL}: ${status}`}
              onClick={props.onRecordChord}
              onKeyDown={onRecorderKey}
            >
              {props.recording ? CHORD_PROMPT : REBIND_LABEL}
            </button>
          }
        />
      </SettingsGroup>

      <SettingsGroup id="projectPage" title="PROJECT PAGE">
        <SwitchRow
          spec={spec('projectpage-roast')}
          checked={props.settings.roastEnabled}
          onChange={(next) => {
            props.onPatch({ roastEnabled: next });
          }}
        />
      </SettingsGroup>
    </>
  );
}
