/**
 * [p3] §30.9's per-check switches — **one row per check, global, in §11.3a's drawer.**
 *
 * The rows are the core's `Settings.healthChecks`, which it emits **in full, one entry per
 * `DebtSource` variant**, so this group holds no list and no count of its own: a written-down one
 * is R22 again, and the day a tenth source lands it would draw nine rows over ten checks.
 *
 * **The observable effect is what makes each row a control and not a statement row:** the
 * check's items leave the project page's list, `eligible` drops, and this group's caption says
 * so. Off hides; it never closes, and a check switched back on reads `unknown` until it next
 * runs — the footnote states that rather than leaving it to be discovered.
 *
 * Each row is named by its `DebtSource` variant, which is the name the `HEALTH` tab gives the
 * same check; a second vocabulary for one check would be two names drifting apart.
 */
import type { ReactElement } from 'react';
import type { HealthCheckSwitch, Settings } from '../../generated/protocol.js';
import { offCause } from '../project/health/checkForms.js';
import { SettingsGroup, SwitchRow, type SettingsRowSpec } from './rows.js';
import { SD } from './styles.js';

export const HEALTH_CHECKS_TITLE = 'HEALTH CHECKS';

export const HEALTH_CHECKS_FOOTNOTE =
  'A check switched off hides its items and is left out of what is counted. It closes nothing: ' +
  'switched back on, it reads unknown until it next runs, and its items return as they were.';

/**
 * R142: `todo_marker` is `off` while the source-reading grant is missing, whatever its switch
 * says, so a row showing *on* over a check that cannot run names what it is waiting for.
 */
export const GRANT_MISSING_NOTE = 'STAYS OFF UNTIL READING SOURCE FILES IS ALLOWED, ABOVE';

/** The template every drawn row clones, as the roots group clones its per-root switches. */
export const HEALTH_GROUP_ROWS: readonly SettingsRowSpec[] = [
  {
    id: 'health-check',
    group: 'healthChecks',
    label: 'Health check',
    note: null,
    backing: { kind: 'command', command: 'settings.set' },
  },
];

export function healthChecksCaption(checks: readonly HealthCheckSwitch[]): string {
  return `${String(checks.filter((c) => c.enabled).length)} OF ${String(checks.length)} ON`;
}

/**
 * The two causes of `off` are told apart in one place, `offCause`; this asks it the same question
 * the `HEALTH` tab does. **The switch wins when both apply**, so a row switched off carries no
 * note about a grant that would change nothing.
 */
function rowNote(entry: HealthCheckSwitch, settings: Settings): string | null {
  if (!entry.enabled) return null;
  const cause = offCause({ id: entry.check, outcome: 'off', unknownReason: null }, settings);
  return cause === 'grantMissing' ? GRANT_MISSING_NOTE : null;
}

export interface HealthGroupsProps {
  readonly settings: Settings;
  /** Sends only the entry it names; `SettingsPatch.healthChecks` applies the entries it carries. */
  readonly onPatch: (patch: Partial<Settings>) => void;
}

export function HealthGroups(props: HealthGroupsProps): ReactElement {
  const template = HEALTH_GROUP_ROWS[0];
  if (template === undefined) throw new Error('no health-check row template');
  const checks = props.settings.healthChecks;
  return (
    <SettingsGroup
      id="healthChecks"
      title={HEALTH_CHECKS_TITLE}
      caption={healthChecksCaption(checks)}
    >
      {checks.map((entry) => (
        <SwitchRow
          key={entry.check}
          spec={{
            ...template,
            id: `${template.id}-${entry.check}`,
            label: entry.check,
            note: rowNote(entry, props.settings),
          }}
          checked={entry.enabled}
          onChange={(next) => {
            props.onPatch({ healthChecks: [{ check: entry.check, enabled: next }] });
          }}
        />
      ))}
      <p style={SD.rowNote}>{HEALTH_CHECKS_FOOTNOTE}</p>
    </SettingsGroup>
  );
}
