/**
 * **AC-P2-20-10's renderer half.** *"Closing and reopening the settings drawer returns to the
 * same pending flow."*
 *
 * That is a **mount-lifecycle** claim, and it is the half the core test cannot reach: a panel
 * that cancelled its flow on unmount would pass `accounts_device_flow.rs` and fail the user.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import type { CommandArgs, CommandName, CommandResult } from '../../generated/protocol';
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
}

/** Records every command, and answers `accounts.connect` the way the core does. */
function fakeCore(): FakeCore {
  const calls: CommandName[] = [];
  // The core returns the live flow on a repeat connect, with the time actually left.
  let remaining = 890;
  const request = <K extends CommandName>(
    name: K,
    args: CommandArgs[K],
  ): Promise<CommandResult[K]> => {
    void args;
    calls.push(name);
    if (name === 'accounts.list') return Promise.resolve([] as unknown as CommandResult[K]);
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
  return { calls, request };
}

describe('the drawer closes without cancelling', () => {
  it('AC-P2-20-10 returns to the same flow with less time left, and cancels nothing', async () => {
    const core = fakeCore();
    const first = render(<GithubPanelHost request={core.request} />);
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
    render(<GithubPanelHost request={core.request} />);
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
    render(<GithubPanelHost request={core.request} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    // Reading the account list is not the same as beginning an OAuth flow.
    expect(core.calls).toContain('accounts.list');
    expect(core.calls).not.toContain('accounts.connect');
  });

  it('stops recovering once the flow is cancelled', async () => {
    const core = fakeCore();
    const first = render(<GithubPanelHost request={core.request} />);
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

    render(<GithubPanelHost request={core.request} />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'CONNECT' })).toBeTruthy();
    });
    expect(screen.queryByText('WXYZ-1234')).toBeNull();
  });
});
