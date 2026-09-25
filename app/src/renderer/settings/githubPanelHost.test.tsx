/**
 * **AC-P2-20-10's renderer half.** *"Closing and reopening the settings drawer returns to the
 * same pending flow."*
 *
 * That is a **mount-lifecycle** claim, and it is the half the core test cannot reach: a panel
 * that cancelled its flow on unmount would pass `accounts_device_flow.rs` and fail the user.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import type { Account, CommandArgs, CommandName, CommandResult } from '../../generated/protocol';
import type { RendererEvent } from '../../shared/channels';
import { GithubPanelHost, resetPendingFlowForTest } from './GithubPanelHost';

afterEach(() => {
  cleanup();
  resetPendingFlowForTest();
});

interface FakeCore {
  readonly calls: CommandName[];
  readonly request: <K extends CommandName>(
    name: K,
    args: CommandArgs[K],
  ) => Promise<CommandResult[K]>;
  /** The `accounts` topic, as the core's publisher drives it. */
  readonly emit: (event: RendererEvent) => void;
  readonly subscribe: (handler: (event: RendererEvent) => void) => () => void;
  /** What `accounts.list` answers from now on. */
  readonly setAccounts: (rows: readonly Account[]) => void;
}

/** Records every command, and answers `accounts.connect` the way the core does. */
function fakeCore(): FakeCore {
  const calls: CommandName[] = [];
  // The core returns the live flow on a repeat connect, with the time actually left.
  let remaining = 890;
  let rows: readonly Account[] = [];
  const handlers = new Set<(event: RendererEvent) => void>();
  const request = <K extends CommandName>(name: K): Promise<CommandResult[K]> => {
    calls.push(name);
    if (name === 'accounts.list') return Promise.resolve(rows as unknown as CommandResult[K]);
    if (name === 'accounts.orgs') return Promise.resolve(null as unknown as CommandResult[K]);
    if (name === 'accounts.connect') {
      remaining -= 30;
      return Promise.resolve({
        userCode: 'WXYZ-1234',
        verificationUri: 'https://forge.example.invalid/login/device',
        expiresInSecs: remaining,
        intervalSecs: 5,
      } as unknown as CommandResult[K]);
    }
    return Promise.resolve(undefined as unknown as CommandResult[K]);
  };
  return {
    calls,
    request,
    emit: (event) => {
      for (const fn of [...handlers]) fn(event);
    },
    subscribe: (handler) => {
      handlers.add(handler);
      return () => handlers.delete(handler);
    },
    setAccounts: (next) => {
      rows = next;
    },
  };
}

/** One connected account, enough for the panel's `CONNECTED` state. */
function connectedAccount(): Account {
  return {
    id: 1 as Account['id'],
    provider: 'github',
    host: 'forge.example.invalid',
    login: 'octo-fixture',
    displayName: null,
    authKind: 'device',
    scopeTier: 'public',
    grantedScopes: [],
    scopesObservedAt: null,
    connectedAt: 1_800_000_000,
    lastVerifiedAt: null,
    lastErrorKind: null,
    lastErrorAt: null,
    enabled: true,
  };
}

describe('the drawer closes without cancelling', () => {
  it('AC-P2-20-10 returns to the same flow with less time left, and cancels nothing', async () => {
    const core = fakeCore();
    const first = render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });

    fireEvent.click(screen.getByRole('button', { name: 'CONNECT' }));
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });
    const shown = screen.getByText(/s left/u).textContent ?? '';

    // Close the drawer. This unmounts the panel; it must not end the flow.
    first.unmount();
    expect(core.calls, 'closing the drawer cancelled the flow').not.toContain(
      'accounts.cancelConnect',
    );

    // Reopen it.
    render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });
    const again = screen.getByText(/s left/u).textContent ?? '';

    expect(again).not.toBe(shown);
    expect(Number.parseInt(again, 10)).toBeLessThan(Number.parseInt(shown, 10));
    expect(core.calls).not.toContain('accounts.cancelConnect');
  });

  it('starts no flow on mount for a user who never asked', async () => {
    const core = fakeCore();
    render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    // Reading the account list is not the same as beginning an OAuth flow.
    expect(core.calls).toContain('accounts.list');
    expect(core.calls).not.toContain('accounts.connect');
  });

  it('stops recovering once the flow is cancelled', async () => {
    const core = fakeCore();
    const first = render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    fireEvent.click(screen.getByRole('button', { name: 'CONNECT' }));
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });
    fireEvent.click(screen.getByRole('button', { name: 'CANCEL' }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    first.unmount();

    render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    expect(screen.queryByText('WXYZ-1234')).toBeNull();
  });
});

/**
 * The core polls the device flow, so the panel only learns it ended from the `accounts` topic.
 * Without that subscription `CONNECTING` had no exit: the panel showed a finished flow's code
 * until it was remounted, and then — still believing a flow was live — called `accounts.connect`
 * and began a **second** one over the account that had just connected.
 */
describe('the panel leaves CONNECTING when the core says the flow ended', () => {
  it('a granted flow becomes the connected account, and reopening starts no second flow', async () => {
    const core = fakeCore();
    const first = render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    fireEvent.click(screen.getByRole('button', { name: 'CONNECT' }));
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });

    // The user authorises in their browser: the core stores the account and says so.
    core.setAccounts([connectedAccount()]);
    core.emit({ topic: 'accounts', event: 'connected', data: connectedAccount() });
    await waitFor(() => {
      expect(screen.queryByText('WXYZ-1234')).toBeNull();
    });
    expect(screen.getByText('octo-fixture', { exact: false })).toBeTruthy();

    const before = core.calls.filter((name) => name === 'accounts.connect').length;
    first.unmount();
    render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByText('octo-fixture', { exact: false })).toBeTruthy();
    });
    expect(
      core.calls.filter((name) => name === 'accounts.connect').length,
      'reopening the drawer began a second device flow over a connected account',
    ).toBe(before);
  });

  it('a flow that expires clears the code rather than showing a dead one', async () => {
    const core = fakeCore();
    render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    fireEvent.click(screen.getByRole('button', { name: 'CONNECT' }));
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });

    core.emit({
      topic: 'accounts',
      event: 'connect_progress',
      data: { stage: 'expired', intervalSecs: 5 },
    });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    expect(screen.queryByText('WXYZ-1234')).toBeNull();
  });

  it('a still-polling stage leaves the pending flow alone', async () => {
    const core = fakeCore();
    render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    fireEvent.click(screen.getByRole('button', { name: 'CONNECT' }));
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });

    // `pending` and `slow_down` are the poll continuing, not the flow ending.
    core.emit({
      topic: 'accounts',
      event: 'connect_progress',
      data: { stage: 'slow_down', intervalSecs: 9 },
    });
    core.emit({ topic: 'scan', event: 'progress', data: { walkedDirs: 1 } });
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });
  });
});

/**
 * **R78's renderer half.** A grant the machine could not store must not read as *nothing
 * happened*: the user has just authorised the application on the provider's own site.
 */
describe('a grant the store refused', () => {
  it('says which side refused, and clears when another flow begins', async () => {
    const core = fakeCore();
    render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    fireEvent.click(screen.getByRole('button', { name: 'CONNECT' }));
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });

    core.emit({
      topic: 'accounts',
      event: 'connect_progress',
      data: {
        stage: 'not_stored',
        intervalSecs: 5,
        reason: 'the keychain refused the operation',
      },
    });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    expect(screen.queryByText('WXYZ-1234')).toBeNull();

    const line = screen.getByText(/the keychain refused the operation/u).textContent ?? '';
    expect(line, 'the line does not say the provider did its part').toContain(
      'granted the token and this machine could not store it',
    );
    expect(line, 'a user who reconnects must be told it is safe').toContain('connecting again');

    // Starting another flow clears it: a stale explanation on a live attempt is its own lie.
    fireEvent.click(screen.getByRole('button', { name: 'CONNECT' }));
    await waitFor(() => {
      expect(screen.getByText('WXYZ-1234')).toBeTruthy();
    });
    expect(screen.queryByText(/the keychain refused the operation/u)).toBeNull();
  });

  it('says nothing when no flow has failed that way', async () => {
    const core = fakeCore();
    render(<GithubPanelHost request={core.request} subscribe={core.subscribe} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    // The ordinary NOT CONNECTED state explains the consequence and nothing else.
    expect(document.querySelector('[data-row="github-not-stored"]')).toBeNull();
  });
});
