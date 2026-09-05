/**
 * §11.3a group 9, as a real control rather than a statement.
 *
 * Three rules hold this file together and each is asserted rather than intended:
 *
 * - **No scope string is written here.** The `NOT CONNECTED` state lists **no scopes at all** —
 *   nothing has been granted, so there is nothing to render, and listing what *would* be
 *   requested is how two non-existent scope names came to be drawn on this very screen. The
 *   chips appear only in `CONNECTED`, where they are a read-back fact from `grantedScopes`.
 * - **No step is a modal.** No `role="dialog"`, no `aria-modal`, no focus trap, no backdrop.
 *   Closing the drawer does not cancel a pending flow — the poll is core-side state.
 * - **Unknown is `—`**, §8.4.1's glyph, and there is no second vocabulary.
 *
 * The verification URI is rendered as **text, not a link**: the external opener is §25's and this
 * plan must not build a second one. The same is true of the provider's revocation page.
 */
import type { ReactElement } from 'react';

import type { Account, AccountOrg, DeviceGrant } from '../../generated/protocol';
import { accountsViewState, UNKNOWN } from './accountsState';
import { SD } from './styles';

/** What the mounting surface supplies. Every one of them acts; none is a statement dressed up. */
export interface GithubPanelProps {
  readonly accounts: readonly Account[];
  readonly pendingGrant: DeviceGrant | null;
  readonly orgs: readonly AccountOrg[] | null;
  readonly onConnect: () => void;
  /**
   * Optional, and absent means the button is **not drawn**. §11.3a's dead-switch rule: the PAT
   * path needs a host field this plan does not draw, and a control wired to a no-op is exactly
   * what that rule forbids. The absent one stays absent.
   */
  readonly onConnectPat?: (() => void) | undefined;
  readonly onCancelConnect: () => void;
  readonly onUpgradeScope: () => void;
  readonly onDisconnect: () => void;
  readonly onSetOrgEnabled: (login: string, enabled: boolean) => void;
  /** R78: why the last flow ended in `not_stored`, or `null` when none did. */
  readonly refusal?: string | null | undefined;
}

/** §20.9: disconnecting does not revoke the grant, and the surface says so plainly. */
export const REVOCATION_CONSEQUENCE =
  'Disconnecting removes the token from this machine. It does not revoke this application on the provider — do that on the provider’s own settings page:';

/**
 * R78's `not_stored`, in words. The forge did its part; this machine did not.
 *
 * It names **which side** refused, because the user has just authorised the application on a
 * third-party site and every other reading of an empty screen is wrong: that they mistyped the
 * code, that the app was denied, that the code ran out.
 */
export const NOT_STORED_CONSEQUENCE =
  'The provider granted the token and this machine could not store it, so nothing was connected. The grant still exists on the provider — connecting again is safe. The reason the keychain gave:';

/** §20.11's consequence, stated where a token would otherwise be assumed. */
export const NOT_CONNECTED_CONSEQUENCE =
  'Stars, issues, pull requests and CI state stay unknown until a token exists — and unknown is drawn as unknown, never as zero. Stored in the OS keychain, never in a config file.';

export function GithubPanel(props: GithubPanelProps): ReactElement {
  const state = accountsViewState(props.accounts, props.pendingGrant, props.orgs, props.refusal ?? null);

  if (state.kind === 'notConnected') {
    return (
      <div style={SD.blockAbsent} data-row="github-not-connected">
        <span style={SD.rowLabel}>NOT CONNECTED</span>
        {state.refusal !== null && (
          <p style={SD.rowNote} data-row="github-not-stored">
            {`${NOT_STORED_CONSEQUENCE} ${state.refusal}`}
          </p>
        )}
        <p style={SD.rowNote}>{NOT_CONNECTED_CONSEQUENCE}</p>
        <div style={SD.rowActions}>
          <button type="button" style={SD.buttonFilled} onClick={props.onConnect}>
            CONNECT
          </button>
          {props.onConnectPat !== undefined && (
            <button type="button" style={SD.buttonOutline} onClick={props.onConnectPat}>
              USE A TOKEN INSTEAD
            </button>
          )}
        </div>
      </div>
    );
  }

  if (state.kind === 'connecting') {
    return (
      <div style={SD.blockAbsent} data-row="github-connecting">
        <span style={SD.rowLabel}>{state.userCode}</span>
        {/* Text, not a link: §25 owns the external opener and this plan may not build a second. */}
        <p style={SD.rowNote}>{state.verificationUri}</p>
        <p style={SD.rowNote}>{`${String(state.secondsLeft)}s left`}</p>
        <div style={SD.rowActions}>
          <button type="button" style={SD.buttonOutline} onClick={props.onCancelConnect}>
            CANCEL
          </button>
        </div>
      </div>
    );
  }

  return (
    <div style={SD.blockAbsent} data-row="github-connected">
      <span style={SD.rowLabel}>{`${state.login} at ${state.host}`}</span>
      <p style={SD.rowNote}>{`${state.tier} · last verified ${state.lastVerified}`}</p>

      {/* Read back from the server, one chip per string. Never written in this file. */}
      <div style={{ ...SD.rowActions, flexWrap: 'wrap', gap: '5px' }} data-row="github-scopes">
        {state.grantedScopes.map((scope) => (
          <span key={scope} style={SD.chip}>
            {scope}
          </span>
        ))}
      </div>

      <div data-row="github-orgs">
        {state.orgs === null ? (
          // Not enumerable under the public tier, which lacks the org-reading scope. `—`,
          // never `0 organizations`, never `failed`. (The scope is not named here: comments
          // survive into the built bundle, and `p2-20-scope-strings` scans that too.)
          <p style={SD.rowNote}>{`Organizations ${UNKNOWN}`}</p>
        ) : (
          state.orgs.map((org) => (
            <div key={org.login} style={SD.row} data-row={`github-org-${org.login}`}>
              <span style={SD.rowLabel}>{org.login}</span>
              <span style={SD.rowNote}>{org.repoCount}</span>
              <div style={SD.rowActions}>
                <button
                  type="button"
                  role="switch"
                  aria-checked={org.enabled}
                  style={SD.buttonSmall}
                  // Operable whatever `repoCount` says: a control disabled by an unknown is a
                  // dead control.
                  onClick={() => {
                    props.onSetOrgEnabled(org.login, !org.enabled);
                  }}
                >
                  {org.enabled ? 'ON' : 'OFF'}
                </button>
              </div>
            </div>
          ))
        )}
      </div>

      <div style={SD.rowActions}>
        {state.canUpgrade && (
          <button type="button" style={SD.buttonOutline} onClick={props.onUpgradeScope}>
            CONNECT PRIVATE AND ORG REPOSITORIES
          </button>
        )}
        <button type="button" style={SD.buttonOutline} onClick={props.onDisconnect}>
          DISCONNECT
        </button>
      </div>
      <p style={SD.rowNote}>{REVOCATION_CONSEQUENCE}</p>
      {/* Text, not a control: the opener is §25's. */}
      <p style={SD.rowNote}>{`https://${state.host}/settings/applications`}</p>
      {state.otherAccounts > 0 && (
        <p style={SD.rowNote} data-row="github-other-accounts">
          {`${String(state.otherAccounts)} more connected`}
        </p>
      )}
    </div>
  );
}
