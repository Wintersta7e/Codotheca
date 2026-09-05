/**
 * The production supplier for §20.12's panel slot.
 *
 * `GithubPanel` is pure over what it is handed; this is the half that talks to the core. It
 * exists because a slot declared with only a test double is the same defect as a trait declared
 * with only a fake — it compiles, its tests pass, and the drawer draws nothing.
 *
 * **It holds no token and cannot.** `accounts.list` returns `Account`, which carries no token
 * field at any nesting depth, and every mutation below names a command the schema declares.
 */
import { useCallback, useEffect, useState, type ReactElement } from 'react';

import type {
  Account,
  AccountOrg,
  CommandArgs,
  CommandName,
  CommandResult,
  ConnectStage,
  DeviceGrant,
} from '../../generated/protocol';
import type { RendererEvent } from '../../shared/channels';
import { GithubPanel } from './accounts';

/**
 * The stages after which no flow is live any more.
 *
 * `pending` and `slow_down` are the poll continuing; every other stage is the core saying the
 * flow is over, whichever way it ended. Without this the panel showed `CONNECTING` for a flow
 * that had already finished, and — because `flowWasStarted` was still true — the next drawer
 * opening called `accounts.connect` again and began a **second** device flow over a connected
 * account.
 */
const TERMINAL_STAGES: readonly ConnectStage[] = ['granted', 'denied', 'expired', 'cancelled'];

/**
 * Whether a Device Flow was started from this process, surviving the panel's unmount.
 *
 * **Closing the settings drawer unmounts the panel; it does not cancel the flow** — the poll is
 * core-side state and `accounts.cancelConnect` or the deadline elapsing are its only two endings.
 * A panel that kept the grant only in component state would show `NOT CONNECTED` on reopen while
 * the core was still polling, which is the surface lying about what is happening.
 *
 * Module-scoped because there is exactly one flow in the process, which is the core's own model.
 * It records only *that* one was started: the grant itself is re-read from the core on remount,
 * so the countdown shown is the time actually left rather than the value first returned.
 */
let flowWasStarted = false;

/** Test seam: a fresh module per test file is not something vitest guarantees. */
export function resetPendingFlowForTest(): void {
  flowWasStarted = false;
}

export interface GithubPanelHostProps {
  readonly request: <K extends CommandName>(
    name: K,
    args: CommandArgs[K],
  ) => Promise<CommandResult[K]>;
  /**
   * The `accounts` topic. **Required**: the core polls the device flow, so this is the only way
   * the panel learns a flow ended, and without it `CONNECTING` is a state with no exit.
   */
  readonly subscribe: (handler: (event: RendererEvent) => void) => () => void;
  /**
   * The PAT path needs a host field, which is a text input this plan does not draw. **Absent
   * means the button is not drawn** — a control wired to a no-op is what §11.3a forbids.
   */
  readonly onConnectPat?: (() => void) | undefined;
}

export function GithubPanelHost(props: GithubPanelHostProps): ReactElement {
  const { request, subscribe } = props;
  const [accounts, setAccounts] = useState<readonly Account[]>([]);
  const [orgs, setOrgs] = useState<readonly AccountOrg[] | null>(null);
  const [grant, setGrant] = useState<DeviceGrant | null>(null);

  /**
   * Re-reads the live grant, if one was started. `accounts.connect` with a flow already pending
   * **returns that flow and starts no second one** (§20.2), so this recovers the same `userCode`
   * with a smaller `expiresInSecs` rather than beginning anything.
   */
  const recoverPendingFlow = useCallback(async (): Promise<void> => {
    if (!flowWasStarted) return;
    setGrant(await request('accounts.connect', {}));
  }, [request]);

  const refresh = useCallback(async (): Promise<void> => {
    const listed = await request('accounts.list', {});
    setAccounts(listed);
    const first = listed.at(0);
    if (first === undefined) {
      setOrgs(null);
      return;
    }
    // `null` is *unknown* — the public tier cannot enumerate orgs — and it is carried through
    // as null rather than flattened to an empty list, which would claim there are none.
    setOrgs(await request('accounts.orgs', { accountId: first.id }));
  }, [request]);

  useEffect(() => {
    void (async () => {
      await refresh();
      await recoverPendingFlow();
    })();
  }, [refresh, recoverPendingFlow]);

  /**
   * The `accounts` topic, which is how a core-side flow reports that it ended.
   *
   * `connected` and a terminal `connect_progress` both mean the same thing here — there is no
   * live flow any more — so both clear the grant and re-read. Re-reading rather than folding the
   * event's payload into state keeps one source for what is connected: the event says *that*
   * something changed, `accounts.list` says what.
   */
  useEffect(
    () =>
      subscribe((event) => {
        if (event.topic !== 'accounts') return;
        const stage = (event.data as { stage?: ConnectStage } | null)?.stage;
        const ended =
          event.event === 'connected' ||
          event.event === 'disconnected' ||
          (event.event === 'connect_progress' &&
            stage !== undefined &&
            TERMINAL_STAGES.includes(stage));
        if (!ended) return;
        flowWasStarted = false;
        setGrant(null);
        void refresh();
      }),
    [subscribe, refresh],
  );

  const first = accounts.at(0);

  return (
    <GithubPanel
      accounts={accounts}
      pendingGrant={grant}
      orgs={orgs}
      onConnect={() => {
        void (async () => {
          flowWasStarted = true;
          setGrant(await request('accounts.connect', {}));
          await refresh();
        })();
      }}
      onConnectPat={props.onConnectPat}
      onCancelConnect={() => {
        void (async () => {
          await request('accounts.cancelConnect', {});
          flowWasStarted = false;
          setGrant(null);
          await refresh();
        })();
      }}
      onUpgradeScope={() => {
        if (first === undefined) return;
        void (async () => {
          setGrant(await request('accounts.upgradeScope', { accountId: first.id }));
        })();
      }}
      onDisconnect={() => {
        if (first === undefined) return;
        void (async () => {
          await request('accounts.disconnect', { accountId: first.id });
          flowWasStarted = false;
          setGrant(null);
          await refresh();
        })();
      }}
      onSetOrgEnabled={(login, enabled) => {
        if (first === undefined) return;
        void (async () => {
          await request('accounts.setOrgEnabled', {
            accountId: first.id,
            orgLogin: login,
            enabled,
          });
          await refresh();
        })();
      }}
    />
  );
}
