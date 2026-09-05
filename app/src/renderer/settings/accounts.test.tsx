/**
 * §20.12's panel, **mounted**. A criterion that makes a claim about a surface needs a test that
 * renders it: `accountsState.test.ts` proves the machine, and none of the rules below — the
 * payload-driven chips, the absence of a modal, the operable toggle under an unknown — is
 * visible from a pure function.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Account, AccountId, AccountOrg, DeviceGrant } from '../../generated/protocol';
import { GithubPanel, type GithubPanelProps } from './accounts';

// The repo's convention: RTL does not auto-clean here, and without this every screen query
// after the first render matches across every mounted tree.
afterEach(cleanup);

const aid = (n: number): AccountId => n as AccountId;

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

function props(over: Partial<GithubPanelProps> = {}): GithubPanelProps {
  return {
    accounts: [],
    pendingGrant: null,
    orgs: null,
    onConnect: vi.fn(),
    onConnectPat: vi.fn(),
    onCancelConnect: vi.fn(),
    onUpgradeScope: vi.fn(),
    onDisconnect: vi.fn(),
    onSetOrgEnabled: vi.fn(),
    ...over,
  };
}

const grant: DeviceGrant = {
  userCode: 'ABCD-EFGH',
  verificationUri: 'https://forge.example.invalid/login/device',
  expiresInSecs: 890,
  intervalSecs: 5,
};

describe('§20.12, mounted', () => {
  it('lists no scope at all when nothing has been granted', () => {
    // Nothing is granted, so there is nothing to render. Listing what *would* be requested is
    // how two scope names that do not exist came to be drawn on this screen.
    const { container } = render(<GithubPanel {...props()} />);
    expect(container.querySelector('[data-row="github-scopes"]')).toBeNull();
    expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'USE A TOKEN INSTEAD' })).toBeTruthy();
  });

  it('draws no token button when nothing answers it', () => {
    // §11.3a's dead-switch rule: the absent slot stays absent rather than becoming a no-op.
    const without = props();
    delete (without as { onConnectPat?: () => void }).onConnectPat;
    render(<GithubPanel {...without} />);
    expect(screen.queryByRole('button', { name: 'USE A TOKEN INSTEAD' })).toBeNull();
    expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
  });

  it('AC-P2-20-3 draws the chips the payload carried, whatever they are', () => {
    // AC-P2-20-3's second half: values that appear in no source file, so a hard-coded list
    // could not produce them.
    const served = ['read:user', 'an:invented:scope'];
    render(<GithubPanel {...props({ accounts: [account({ grantedScopes: served })] })} />);
    for (const scope of served) expect(screen.getByText(scope)).toBeTruthy();
  });

  it('renders an unenumerable org list as unknown and never as zero', () => {
    render(<GithubPanel {...props({ accounts: [account()], orgs: null })} />);
    expect(screen.getByText('Organizations —')).toBeTruthy();
    expect(screen.queryByText(/0 organi/iu)).toBeNull();
    expect(screen.queryByText(/failed/iu)).toBeNull();
  });

  it('keeps an org toggle operable when its count is unknown', () => {
    const onSetOrgEnabled = vi.fn();
    const org: AccountOrg = {
      accountId: aid(1),
      login: 'an-org',
      enabled: false,
      repoCountSeen: null,
      ssoState: null,
      observedAt: null,
    };
    render(
      <GithubPanel
        {...props({
          accounts: [account({ scopeTier: 'private' })],
          orgs: [org],
          onSetOrgEnabled,
        })}
      />,
    );
    expect(screen.getByText('—')).toBeTruthy();
    // A control disabled by an unknown is a dead control.
    const toggle = screen.getByRole('switch');
    expect(toggle.hasAttribute('disabled')).toBe(false);
    fireEvent.click(toggle);
    expect(onSetOrgEnabled).toHaveBeenCalledWith('an-org', true);
  });

  it('renders the user code verbatim, including a length the UI does not expect', () => {
    render(<GithubPanel {...props({ pendingGrant: { ...grant, userCode: 'WXYZ12345678901' } })} />);
    expect(screen.getByText('WXYZ12345678901')).toBeTruthy();
  });

  it('renders a two-account payload without throwing and says a second exists', () => {
    // The one that catches an unguarded `[0]`. A one-account test never can.
    render(
      <GithubPanel
        {...props({ accounts: [account(), account({ id: aid(2), login: 'other' })] })}
      />,
    );
    expect(screen.getByText('octo at forge.example.invalid')).toBeTruthy();
    expect(screen.getByText('1 more connected')).toBeTruthy();
  });

  it('is not a modal in any of its three states', () => {
    for (const p of [props(), props({ pendingGrant: grant }), props({ accounts: [account()] })]) {
      const { container, unmount } = render(<GithubPanel {...p} />);
      expect(container.querySelector('[role="dialog"]')).toBeNull();
      expect(container.querySelector('[aria-modal]')).toBeNull();
      // No backdrop, and nothing that traps focus.
      expect(container.querySelector('[data-backdrop]')).toBeNull();
      unmount();
    }
  });

  it('renders the verification URI as text, never as a link', () => {
    // A9 gives the external opener to §25; this plan must not build a second one.
    const { container } = render(<GithubPanel {...props({ pendingGrant: grant })} />);
    expect(screen.getByText(grant.verificationUri)).toBeTruthy();
    expect(container.querySelector('a')).toBeNull();
    expect(container.querySelector('[role="link"]')).toBeNull();
  });

  it('says the revocation page is elsewhere, and offers no control that claims to revoke', () => {
    const { container } = render(<GithubPanel {...props({ accounts: [account()] })} />);
    expect(screen.getByText(/does not revoke this application/u)).toBeTruthy();
    expect(container.querySelector('a')).toBeNull();
  });

  it('offers the upgrade from the public tier only, and never a downgrade', () => {
    const { unmount } = render(<GithubPanel {...props({ accounts: [account()] })} />);
    expect(screen.getByRole('button', { name: /CONNECT PRIVATE AND ORG/u })).toBeTruthy();
    unmount();

    render(<GithubPanel {...props({ accounts: [account({ scopeTier: 'private' })], orgs: [] })} />);
    expect(screen.queryByRole('button', { name: /CONNECT PRIVATE AND ORG/u })).toBeNull();
    // There is no in-place downgrade: a button that might narrow nothing is worse than none.
    expect(screen.queryByRole('button', { name: /DOWNGRADE|NARROW|PUBLIC ONLY/iu })).toBeNull();
  });

  it('gives every drawn button a handler', () => {
    const handlers = {
      onConnect: vi.fn(),
      onConnectPat: vi.fn(),
      onCancelConnect: vi.fn(),
      onUpgradeScope: vi.fn(),
      onDisconnect: vi.fn(),
      onSetOrgEnabled: vi.fn(),
    };
    const { container } = render(<GithubPanel {...props(handlers)} />);
    const buttons = [...container.querySelectorAll('button')];
    expect(buttons.length).toBeGreaterThan(0);
    for (const button of buttons) fireEvent.click(button);
    // Every button in the NOT CONNECTED state did something.
    expect(handlers.onConnect).toHaveBeenCalled();
    expect(handlers.onConnectPat).toHaveBeenCalled();
  });
});
