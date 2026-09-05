/**
 * §11.3a's dead-switch rule: *"Every switch must do something observable or be presented as a
 * statement rather than a control."* The two variants below are the whole vocabulary, and there
 * is deliberately no disabled variant — a disabled switch still announces as a switch (§11.7),
 * which is the same defect one layer down.
 *
 * The rule is carried by the type rather than remembered:
 *  - a control cannot exist without a handler, because `SwitchRowProps` is a union whose
 *    statement arm has no `onChange` to pass and there is no third arm;
 *  - a control must name what it does, because its spec's `backing` names a `CommandName`, a
 *    shell channel or a host slot, and `deadSwitch.test.tsx` walks the registry checking each;
 *  - a statement is a `<span>` with no role, no tab stop and no `aria-*` state.
 */
import type { ReactElement, ReactNode } from 'react';
import type { CommandName } from '../../generated/protocol.js';
import { SD } from './styles.js';

export type SettingsGroupId =
  | 'roots'
  | 'targets'
  | 'excluded'
  | 'scanning'
  | 'motion'
  | 'residency'
  | 'projectPage'
  | 'data'
  | 'identity'
  | 'github'
  | 'notifications';

/** §11.3a's table, in its order: 1–9 with 8a between 8 and 9, then the notification block. */
export const SETTINGS_GROUP_ORDER: readonly SettingsGroupId[] = [
  'roots',
  'targets',
  'excluded',
  'scanning',
  'motion',
  'residency',
  'projectPage',
  'data',
  'identity',
  'github',
  'notifications',
];

/**
 * What the mounting surface supplies, and what the drawer therefore draws. The drawer holds no
 * project picker, no executable picker and no §1.4 identity card, and phase 1 ships none of the
 * three from here — so each row is drawn with its slot or not at all, which is §11.3a's second
 * limb. Typed rather than `unknown`, so a missing slot is a compile error at the mount point
 * rather than a cast at the call site.
 */
export interface SettingsSlots {
  readonly chooseProjectToHide?: () => void;
  readonly chooseLaunchTarget?: (language: string) => void;
  readonly identityCard?: () => ReactElement | null;
  readonly addIdentityAddress?: () => void;
  /** [p2] §20.12's account panel, on `identityCard`'s precedent: `groupsData.tsx` stays free of
   *  protocol calls and the mounting surface owns the commands. */
  readonly githubPanel?: () => ReactElement | null;
}

export type SettingsSlotName = keyof SettingsSlots;

export type RowBacking =
  | { readonly kind: 'command'; readonly command: CommandName }
  | { readonly kind: 'shell'; readonly channel: string }
  | { readonly kind: 'host'; readonly slot: SettingsSlotName }
  | { readonly kind: 'statement' };

export interface SettingsRowSpec {
  readonly id: string;
  readonly group: SettingsGroupId;
  readonly label: string;
  readonly note: string | null;
  readonly backing: RowBacking;
}

export function isControl(backing: RowBacking): boolean {
  return backing.kind !== 'statement';
}

export interface SettingsGroupProps {
  readonly id: SettingsGroupId;
  readonly title: string;
  readonly caption?: string;
  readonly children: ReactNode;
}

export function SettingsGroup(props: SettingsGroupProps): ReactElement {
  const titleId = `sd-group-${props.id}`;
  return (
    <section style={SD.group} role="group" aria-labelledby={titleId} data-group={props.id}>
      <div style={SD.groupHeader}>
        <h2 id={titleId} style={SD.groupTitle}>
          {props.title}
        </h2>
        <span style={SD.groupRule} aria-hidden="true" />
        {props.caption !== undefined && <span style={SD.groupCaption}>{props.caption}</span>}
      </div>
      <div style={SD.rows}>{props.children}</div>
    </section>
  );
}

export interface SettingsRowProps {
  readonly spec: SettingsRowSpec;
  /** The switch or statement mark, which the design puts before the text, never after it. */
  readonly lead?: ReactNode;
  readonly value?: ReactNode;
  readonly control?: ReactNode;
}

export function SettingsRow(props: SettingsRowProps): ReactElement {
  return (
    <div style={SD.row} data-row={props.spec.id}>
      {props.lead}
      <div style={SD.rowText}>
        <span style={SD.rowLabel}>{props.spec.label}</span>
        {props.value !== undefined && <span style={SD.rowValue}>{props.value}</span>}
        {props.spec.note !== null && <span style={SD.rowNote}>{props.spec.note}</span>}
      </div>
      {props.control !== undefined && <div style={SD.rowActions}>{props.control}</div>}
    </div>
  );
}

export type SwitchRowProps =
  | {
      readonly spec: SettingsRowSpec;
      readonly checked: boolean;
      readonly onChange: (next: boolean) => void;
      readonly value?: ReactNode;
    }
  | { readonly spec: SettingsRowSpec; readonly on: boolean; readonly value?: ReactNode };

export function SwitchRow(props: SwitchRowProps): ReactElement {
  if ('onChange' in props) {
    const { spec, checked, onChange } = props;
    return (
      <SettingsRow
        spec={spec}
        {...(props.value === undefined ? {} : { value: props.value })}
        lead={
          <button
            type="button"
            role="switch"
            aria-checked={checked}
            aria-label={spec.label}
            style={checked ? { ...SD.switchTrack, ...SD.switchTrackOn } : SD.switchTrack}
            onClick={() => {
              onChange(!checked);
            }}
          >
            <span
              style={checked ? { ...SD.switchKnob, ...SD.switchKnobOn } : SD.switchKnob}
              aria-hidden="true"
            />
          </button>
        }
      />
    );
  }
  // The statement variant. A span, not a control: no role, no tabindex, no aria-checked and no
  // aria-disabled. The mark is decoration and is hidden from the accessibility tree.
  return (
    <SettingsRow
      spec={props.spec}
      {...(props.value === undefined ? {} : { value: props.value })}
      lead={<span style={SD.statementTrack} aria-hidden="true" />}
    />
  );
}
