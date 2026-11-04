/**
 * §43.11's drawer group, after `MOTION`: one real switch for quiet mode and the two target rows
 * §39.12 stores and gives no home of its own.
 *
 * - **The switch** is `Settings.motivationShown`; checked means shown, and quiet is the switch off.
 *   Its note is the closed list, read from `shared/motivationQuiet.ts`.
 * - **Directly below it** is where the quests switch goes when quests land; this group leaves the
 *   position and draws no row for it.
 * - **The targets** are steppers bounded by the core's own ranges (`shared/ringTargets.ts`). A step
 *   past a bound is absent rather than disabled, as the reroll stepper's back step is, because a
 *   greyed control is the dead control §11.3a forbids. An unset rhythm target reads *not set*,
 *   never `0`. Both rows are a game's settings, so quiet hides them; the switch stays, or quiet
 *   could not be left.
 */
import type { ReactElement } from 'react';

import type { Settings } from '../../generated/protocol.js';
import { DATA_MOTIVATION, type QuietEntryId } from '../../shared/motivationQuiet.js';
import {
  FOCUS_TARGET_MAX_MINUTES,
  FOCUS_TARGET_MIN_MINUTES,
  RHYTHM_TARGET_MAX_DAYS,
  RHYTHM_TARGET_MIN_DAYS,
} from '../../shared/ringTargets.js';
import {
  FOCUS_TARGET_LABEL,
  QUIET_GROUP_TITLE,
  QUIET_SWITCH_LABEL,
  QUIET_SWITCH_NOTE,
  RHYTHM_TARGET_LABEL,
} from '../motivation/copy.js';
import { TARGET_UNSET } from './groupsScan.js';
import { SettingsGroup, SettingsRow, SwitchRow, type SettingsRowSpec } from './rows.js';
import { SD } from './styles.js';

/** One press of the Focus stepper. The default (45) and both bounds sit on this grid. */
export const FOCUS_TARGET_STEP_MINUTES = 15;

export const MOTIVATION_GROUP_ROWS: readonly SettingsRowSpec[] = [
  {
    id: 'motivation-shown',
    group: 'motivation',
    label: QUIET_SWITCH_LABEL,
    note: QUIET_SWITCH_NOTE,
    backing: { kind: 'command', command: 'settings.set' },
  },
  {
    id: 'motivation-focus-target',
    group: 'motivation',
    label: FOCUS_TARGET_LABEL,
    note: null,
    backing: { kind: 'command', command: 'settings.set' },
  },
  {
    id: 'motivation-rhythm-target',
    group: 'motivation',
    label: RHYTHM_TARGET_LABEL,
    note: null,
    backing: { kind: 'command', command: 'settings.set' },
  },
];

const spec = (id: string): SettingsRowSpec => {
  const found = MOTIVATION_GROUP_ROWS.find((row) => row.id === id);
  if (found === undefined) throw new Error(`no settings row named ${id}`);
  return found;
};

/** The attribute that tells quiet mode's count this element is the named entry's. */
const covered = (id: QuietEntryId): Record<string, QuietEntryId> => ({ [DATA_MOTIVATION]: id });

interface StepperProps {
  readonly spec: SettingsRowSpec;
  readonly entry: QuietEntryId;
  /** What the row shows: the value, or *not set*. */
  readonly shown: string;
  /** The value one step down, or `null` where no step stays inside the range. */
  readonly less: number | null;
  readonly more: number | null;
  readonly onStep: (value: number) => void;
}

function StepperRow(props: StepperProps): ReactElement {
  const step = (direction: 'less' | 'more', value: number | null): ReactElement | null =>
    value === null ? null : (
      <button
        type="button"
        data-step={direction}
        style={SD.buttonSmall}
        aria-label={`${props.spec.label} ${direction === 'less' ? '−' : '+'}`}
        onClick={() => {
          props.onStep(value);
        }}
      >
        {direction === 'less' ? '−' : '+'}
      </button>
    );
  return (
    <div style={{ display: 'contents' }} {...covered(props.entry)}>
      <SettingsRow
        spec={props.spec}
        value={<span data-value="">{props.shown}</span>}
        control={
          <>
            {step('less', props.less)}
            {step('more', props.more)}
          </>
        }
      />
    </div>
  );
}

/** One Focus step from `minutes`, clamped onto the range; `null` at the bound it would pass. */
export function focusStep(minutes: number, direction: -1 | 1): number | null {
  if (direction < 0 && minutes <= FOCUS_TARGET_MIN_MINUTES) return null;
  if (direction > 0 && minutes >= FOCUS_TARGET_MAX_MINUTES) return null;
  const next = minutes + direction * FOCUS_TARGET_STEP_MINUTES;
  return Math.min(FOCUS_TARGET_MAX_MINUTES, Math.max(FOCUS_TARGET_MIN_MINUTES, next));
}

/** One rhythm step from `days`; from *not set* the only step is up, to the first day. */
export function rhythmStep(days: number | null, direction: -1 | 1): number | null {
  if (days === null) return direction > 0 ? RHYTHM_TARGET_MIN_DAYS : null;
  const next = days + direction;
  return next < RHYTHM_TARGET_MIN_DAYS || next > RHYTHM_TARGET_MAX_DAYS ? null : next;
}

export interface MotivationGroupsProps {
  readonly settings: Settings;
  readonly onPatch: (patch: Partial<Settings>) => void;
  /** Whether the motivation layer is shown; quiet (`false`) hides the target rows. */
  readonly shown: boolean;
}

export function MotivationGroups(props: MotivationGroupsProps): ReactElement {
  const { settings, onPatch } = props;
  return (
    <SettingsGroup id="motivation" title={QUIET_GROUP_TITLE}>
      <SwitchRow
        spec={spec('motivation-shown')}
        checked={settings.motivationShown}
        onChange={(next) => {
          onPatch({ motivationShown: next });
        }}
      />
      {/* The quests switch takes this position, directly below the quiet switch. */}
      {props.shown && (
        <>
          <StepperRow
            spec={spec('motivation-focus-target')}
            entry="rings"
            shown={String(settings.focusTargetMinutes)}
            less={focusStep(settings.focusTargetMinutes, -1)}
            more={focusStep(settings.focusTargetMinutes, 1)}
            onStep={(focusTargetMinutes) => {
              onPatch({ focusTargetMinutes });
            }}
          />
          <StepperRow
            spec={spec('motivation-rhythm-target')}
            entry="rhythm"
            shown={settings.rhythmTarget === null ? TARGET_UNSET : String(settings.rhythmTarget)}
            less={rhythmStep(settings.rhythmTarget, -1)}
            more={rhythmStep(settings.rhythmTarget, 1)}
            onStep={(rhythmTarget) => {
              onPatch({ rhythmTarget });
            }}
          />
        </>
      )}
    </SettingsGroup>
  );
}
