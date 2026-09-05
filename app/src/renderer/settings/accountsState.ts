/**
 * §20.12's three states, as a pure function of what the core answered.
 *
 * It is pure so the states are tested without React, and so the two rules that are easiest to
 * break by accident live in one place:
 *
 * - **Unknown is `—`, and there is no second vocabulary.** Under the public tier the org list is
 *   `null` — *not enumerable*, lacking the org-reading scope — and it renders `—`, never
 *   `0 organizations` and never `failed`.
 *   `[]` is a different claim: enumerated, none found.
 * - **No scope string is written here.** The chips come from `grantedScopes`, which the core read
 *   back from the server. A hard-coded list is how two scope names that do not exist came to be
 *   drawn on a settings screen.
 */
import type { Account, AccountOrg, DeviceGrant } from '../../generated/protocol';

/** §8.4.1's glyph. One vocabulary for unknown, not a second one for this screen. */
export const UNKNOWN = '—';

export interface ConnectedOrg {
  readonly login: string;
  readonly enabled: boolean;
  /** Already rendered: the count, or `—`. A caller may not re-derive it and get `0`. */
  readonly repoCount: string;
}

export type AccountsViewState =
  | { readonly kind: 'notConnected' }
  | {
      readonly kind: 'connecting';
      /** Byte-identical to the server's. Never reformatted, never masked to a fixed width. */
      readonly userCode: string;
      readonly verificationUri: string;
      readonly secondsLeft: number;
    }
  | {
      readonly kind: 'connected';
      readonly login: string;
      readonly host: string;
      readonly tier: 'public' | 'private';
      /** From the payload, never from source. */
      readonly grantedScopes: readonly string[];
      /** An observation time, or `—` when it is NULL. Never a zero and never a date invented. */
      readonly lastVerified: string;
      /** `null` is *unknown*: the org list could not be enumerated. `[]` is *none found*. */
      readonly orgs: readonly ConnectedOrg[] | null;
      /** True only under the public tier, where an upgrade is the thing that can be offered. */
      readonly canUpgrade: boolean;
      /** How many further accounts exist. Phase 2 renders the first row and hides none of them. */
      readonly otherAccounts: number;
    };

/** Seconds, floored at zero: a negative countdown is a deadline already past, not a number. */
function secondsLeft(grant: DeviceGrant): number {
  return Math.max(0, grant.expiresInSecs);
}

function ageLine(seconds: number | null): string {
  return seconds === null ? UNKNOWN : new Date(seconds * 1000).toISOString().slice(0, 10);
}

/**
 * The whole surface, from what the core answered.
 *
 * `accounts` is an **array** and this takes all of it: the renderer may never index `[0]`
 * unguarded, and a two-account payload has to render the first row without throwing and without
 * hiding that a second exists. A one-account test can never catch that.
 */
export function accountsViewState(
  accounts: readonly Account[],
  pendingGrant: DeviceGrant | null,
  orgs: readonly AccountOrg[] | null,
): AccountsViewState {
  const first = accounts.at(0);
  if (first === undefined) {
    // A pending flow with no account yet is the connecting state; otherwise nothing is connected.
    return pendingGrant === null
      ? { kind: 'notConnected' }
      : {
          kind: 'connecting',
          userCode: pendingGrant.userCode,
          verificationUri: pendingGrant.verificationUri,
          secondsLeft: secondsLeft(pendingGrant),
        };
  }

  return {
    kind: 'connected',
    login: first.login,
    host: first.host,
    tier: first.scopeTier,
    grantedScopes: first.grantedScopes,
    lastVerified: ageLine(first.lastVerifiedAt),
    orgs:
      orgs === null
        ? null
        : orgs.map((org) => ({
            login: org.login,
            enabled: org.enabled,
            // NULL is unknown and renders `—`. The toggle still works: a control disabled by an
            // unknown is a dead control.
            repoCount: org.repoCountSeen === null ? UNKNOWN : String(org.repoCountSeen),
          })),
    canUpgrade: first.scopeTier === 'public',
    otherAccounts: accounts.length - 1,
  };
}
