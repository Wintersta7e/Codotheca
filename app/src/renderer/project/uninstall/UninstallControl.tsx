/**
 * §24.8's control. **Disabled with its reasons named, and no override anywhere.**
 *
 * The control is arrangement; every sentence lives in `uninstallCopy.ts`. There is no
 * "uninstall anyway", no context menu, no keyboard path and no hidden affordance — a disposition
 * that is not `safe` renders nothing that can reach the channel, which is what
 * `uninstallControl.test.tsx` queries the DOM for rather than checking the visible button.
 */
import type { ReactElement } from 'react';

import type { UninstallVerdict } from '../../../generated/protocol';
import {
  blockerSentence,
  CHECKING_NOTE,
  dispositionHeading,
  TRASH_AVAILABLE_NOTE,
  TRASH_UNAVAILABLE_NOTE,
  UNINSTALL_LABEL,
} from './uninstallCopy';

export interface UninstallControlProps {
  /** `null` while the verdict is in flight. It never renders enabled-then-disabled. */
  readonly verdict: UninstallVerdict | null;
  readonly onUninstall: () => void;
}

export function UninstallControl({ verdict, onUninstall }: UninstallControlProps): ReactElement {
  if (verdict === null) {
    return (
      <div className="cp-uninstall" data-state="checking">
        <p className="cp-uninstall__note">{CHECKING_NOTE}</p>
      </div>
    );
  }

  const safe = verdict.disposition === 'safe';
  return (
    <div className="cp-uninstall" data-state={verdict.disposition}>
      <p className="cp-uninstall__heading">{dispositionHeading(verdict.disposition)}</p>
      {verdict.blockers.length === 0 ? null : (
        <ul className="cp-uninstall__reasons">
          {verdict.blockers.map((blocker) => (
            <li key={blocker} data-blocker={blocker}>
              {blockerSentence(blocker)}
            </li>
          ))}
        </ul>
      )}
      {/* §24.7F: the copy says what will happen to it **before** the click. */}
      {safe ? (
        <p className="cp-uninstall__fate">
          {verdict.trashAvailable ? TRASH_AVAILABLE_NOTE : TRASH_UNAVAILABLE_NOTE}
        </p>
      ) : null}
      {/* The button exists only when the disposition is `safe`. Rendering it disabled would put
          an element on the page that a script or a stray keyboard path could still activate. */}
      {safe ? (
        <button type="button" className="cp-uninstall__go" onClick={onUninstall}>
          {UNINSTALL_LABEL}
        </button>
      ) : null}
    </div>
  );
}
