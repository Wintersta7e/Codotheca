/**
 * [p3] §29.8's in-context ask, **on the one check that needs a grant** (R137).
 *
 * §29.8 puts the content-scan ask *"the first time a surface would show a debt item"*, and in
 * wave 1 no such surface existed — so p3-29 landed the grant as a reachable control in §11.3's
 * settings group and built no modal and no first-run row. §30.7's `HEALTH` tab is the first
 * surface in the wave order that renders a debt item, so the ask is here.
 *
 * **What this is not.** No modal at launch, no first-run row, and no fourth edit to
 * `app/src/renderer/firstrun/copy.ts`: §29.8's three rendered sites moved together in p3-29's
 * change, and a fourth hand on one promise is how a promise drifts. **Criterion 12 stays
 * unmoved**, and `AC-P3-29-20` is the assertion that says so.
 *
 * **No new command.** The grant is `SettingsPatch.contentScanEnabled`, applied through the
 * existing `settings.set`.
 */
import type { ReactElement } from 'react';

export interface GrantAskProps {
  /** Applies `SettingsPatch.contentScanEnabled = true` through `settings.set`. */
  readonly onGrant: () => void;
}

export const GRANT_ASK_SENTENCE =
  'This check needs permission to read your source files, which has not been given.';
export const GRANT_ASK_ACTION = 'ALLOW READING SOURCE FILES';

export function GrantAsk({ onGrant }: GrantAskProps): ReactElement {
  return (
    <div className="cp-health-ask" data-testid="cp-health-ask">
      <p className="cp-health-ask-line">{GRANT_ASK_SENTENCE}</p>
      <button type="button" className="cp-health-ask-action" onClick={onGrant}>
        {GRANT_ASK_ACTION}
      </button>
    </div>
  );
}
