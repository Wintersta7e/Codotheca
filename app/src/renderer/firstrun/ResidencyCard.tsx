/**
 * §11.3a's ask, inside §8.0's box at priority 5.
 *
 * Its answer is what stamps `first_run_completed_at`, which is what arms §10.5a's `NEW` chip —
 * so both buttons answer and neither closes the row silently. The numbers are the measured ones
 * and live in `copy.ts`: the "~30 MB" they replaced was false by seventeen times, and it reached
 * consent copy once already.
 */
import type { ReactElement } from 'react';
import type { SettingsSetArgs } from '../../generated/protocol';
import { residencyAutostart } from './notices';
import type { ResidencyAnswer } from './notices';
import {
  RESIDENCY_BODY,
  RESIDENCY_NO,
  RESIDENCY_NOTE,
  RESIDENCY_TITLE,
  RESIDENCY_YES,
} from './copy';

/** The two answers this card can produce. Dismissal is the slot's and is the third. */
export type ResidencyCardAnswer = Extract<ResidencyAnswer, 'startWithTheSystem' | 'leaveItOff'>;

/**
 * The whole `settings.set` call an answer makes — the envelope as well as the patch, so the
 * renderer cannot send a well-formed patch under the wrong key.
 *
 * `autostart` is named for every answer, `leaveItOff` included, because the core stamps on the
 * field being **present** in the patch — the ask having been *answered* is the event §11.3a
 * records, not the value chosen. Nothing else is disturbed: a patch that named a second setting
 * would make one answer change something the user did not ask about.
 */
export function residencySettingsArgs(answer: ResidencyAnswer): SettingsSetArgs {
  return {
    patch: {
      effectsTier: null,
      reducedMotionOverride: null,
      autostart: residencyAutostart(answer),
      residentShortcut: null,
      roastEnabled: null,
      logLevel: null,
      installRootId: null,
      contentScanEnabled: null,
      healthChecks: null,
    },
  };
}

export interface ResidencyCardProps {
  readonly onAnswer: (answer: ResidencyCardAnswer) => void;
}

export function ResidencyCard(props: ResidencyCardProps): ReactElement {
  return (
    <div className="cdt-fr-residency">
      <p className="cdt-shelf-notice-title">{RESIDENCY_TITLE}</p>
      <p className="cdt-shelf-notice-body">{RESIDENCY_BODY}</p>
      <p className="cdt-fr-residency-figures">{RESIDENCY_NOTE}</p>
      <div className="cdt-shelf-notice-actions">
        <button
          type="button"
          className="cdt-shelf-notice-primary"
          onClick={() => {
            props.onAnswer('startWithTheSystem');
          }}
        >
          {RESIDENCY_YES}
        </button>
        <button
          type="button"
          className="cdt-shelf-notice-secondary"
          onClick={() => {
            props.onAnswer('leaveItOff');
          }}
        >
          {RESIDENCY_NO}
        </button>
      </div>
    </div>
  );
}
