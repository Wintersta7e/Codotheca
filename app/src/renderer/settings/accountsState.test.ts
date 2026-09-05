import { describe, expect, it } from 'vitest';

import type { Account, AccountId, AccountOrg, DeviceGrant } from '../../generated/protocol';

const aid = (n: number): AccountId => n as AccountId;
import { accountsViewState, UNKNOWN } from './accountsState';

function account(over: Partial<Account> = {}): Account {
  return {
    id: aid(1),
    provider: 'github',
    host: 'forge.example.invalid',
    login: 'octo',
    displayName: null,
    authKind: 'device',
    scopeTier: 'public',
    grantedScopes: [],
    scopesObservedAt: null,
    connectedAt: 1000,
    lastVerifiedAt: null,
    lastErrorKind: null,
    lastErrorAt: null,
    enabled: true,
    ...over,
  };
}

function org(over: Partial<AccountOrg> = {}): AccountOrg {
  return {
    accountId: aid(1),
    login: 'an-org',
    enabled: false,
    repoCountSeen: null,
    ssoState: null,
    observedAt: null,
    ...over,
  };
}

const grant: DeviceGrant = {
  userCode: 'ABCD-EFGH',
  verificationUri: 'https://forge.example.invalid/login/device',
  expiresInSecs: 890,
  intervalSecs: 5,
};

describe('§20.12 as a pure function', () => {
  it('is notConnected with no accounts and no pending flow', () => {
    expect(accountsViewState([], null, null)).toEqual({ kind: 'notConnected', refusal: null });
  });

  // R78: a flow that ended in `not_stored` leaves something to say, and `notConnected` with no
  // explanation is §10.1a's silent no.
  it('carries the refusal when the last flow ended in not_stored', () => {
    expect(accountsViewState([], null, null, 'the keychain refused the operation')).toEqual({
      kind: 'notConnected',
      refusal: 'the keychain refused the operation',
    });
  });

  it('carries the user code unmodified, including a length the UI does not expect', () => {
    // Not eight characters, not hyphenated the way today's provider hyphenates. The "eight
    // character code" describes what the provider returns today; it is not a validator.
    const odd = { ...grant, userCode: 'WXYZ12345678901' };
    const state = accountsViewState([], odd, null);
    expect(state.kind).toBe('connecting');
    if (state.kind !== 'connecting') throw new Error('unreachable');
    expect(state.userCode).toBe('WXYZ12345678901');
    expect(state.secondsLeft).toBe(890);
  });

  it('renders a null lastVerifiedAt as unknown, never as a date it invented', () => {
    const state = accountsViewState([account()], null, null);
    if (state.kind !== 'connected') throw new Error('unreachable');
    expect(state.lastVerified).toBe(UNKNOWN);
  });

  it('AC-P2-20-8 renders an unenumerable org list as unknown and NOT as zero', () => {
    // `orgs === null` is the public tier: no `read:org`, so the list is not enumerable.
    const state = accountsViewState([account()], null, null);
    if (state.kind !== 'connected') throw new Error('unreachable');
    expect(state.orgs).toBeNull();
    // The distinction the whole rule turns on: null is not [].
    expect(state.orgs).not.toEqual([]);
  });

  it('renders an enumerated empty org list as empty, which is a different claim', () => {
    const state = accountsViewState([account({ scopeTier: 'private' })], null, []);
    if (state.kind !== 'connected') throw new Error('unreachable');
    expect(state.orgs).toEqual([]);
    expect(state.orgs).not.toBeNull();
  });

  it('renders an uncounted org as unknown and keeps it togglable', () => {
    const state = accountsViewState([account({ scopeTier: 'private' })], null, [
      org({ repoCountSeen: null }),
      org({ login: 'other', repoCountSeen: 4, enabled: true }),
    ]);
    if (state.kind !== 'connected' || state.orgs === null) throw new Error('unreachable');
    expect(state.orgs[0]?.repoCount).toBe(UNKNOWN);
    expect(state.orgs[0]?.enabled).toBe(false);
    expect(state.orgs[1]?.repoCount).toBe('4');
  });

  it('takes the whole array and reports that a second account exists', () => {
    // A one-account test can never catch an unguarded `[0]`; this is the one that can.
    const state = accountsViewState(
      [account(), account({ id: aid(2), login: 'other' })],
      null,
      null,
    );
    if (state.kind !== 'connected') throw new Error('unreachable');
    expect(state.login).toBe('octo');
    expect(state.otherAccounts).toBe(1);
  });

  it('offers the upgrade only from the public tier', () => {
    const pub = accountsViewState([account({ scopeTier: 'public' })], null, null);
    const priv = accountsViewState([account({ scopeTier: 'private' })], null, []);
    if (pub.kind !== 'connected' || priv.kind !== 'connected') throw new Error('unreachable');
    expect(pub.canUpgrade).toBe(true);
    // There is no in-place downgrade: a client cannot narrow a grant it already has.
    expect(priv.canUpgrade).toBe(false);
  });

  it('renders the scopes the payload carried, whatever they are', () => {
    // Values that appear in no source file, so a hard-coded list cannot produce them.
    const served = ['read:user', 'an:invented:scope'];
    const state = accountsViewState([account({ grantedScopes: served })], null, null);
    if (state.kind !== 'connected') throw new Error('unreachable');
    expect(state.grantedScopes).toEqual(served);
  });

  it('never returns a negative countdown', () => {
    const state = accountsViewState([], { ...grant, expiresInSecs: -30 }, null);
    if (state.kind !== 'connecting') throw new Error('unreachable');
    expect(state.secondsLeft).toBe(0);
  });
});
