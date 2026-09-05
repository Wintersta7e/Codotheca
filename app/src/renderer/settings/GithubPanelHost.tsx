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
  DeviceGrant,
} from '../../generated/protocol';
import { GithubPanel } from './accounts';

export interface GithubPanelHostProps {
  readonly request: <K extends CommandName>(
    name: K,
    args: CommandArgs[K],
  ) => Promise<CommandResult[K]>;
  /**
   * The PAT path needs a host field, which is a text input this plan does not draw. **Absent
   * means the button is not drawn** — a control wired to a no-op is what §11.3a forbids.
   */
  readonly onConnectPat?: (() => void) | undefined;
}

export function GithubPanelHost(props: GithubPanelHostProps): ReactElement {
  const { request } = props;
  const [accounts, setAccounts] = useState<readonly Account[]>([]);
  const [orgs, setOrgs] = useState<readonly AccountOrg[] | null>(null);
  const [grant, setGrant] = useState<DeviceGrant | null>(null);

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
    void refresh();
  }, [refresh]);

  const first = accounts.at(0);

  return (
    <GithubPanel
      accounts={accounts}
      pendingGrant={grant}
      orgs={orgs}
      onConnect={() => {
        void (async () => {
          setGrant(await request('accounts.connect', {}));
          await refresh();
        })();
      }}
      onConnectPat={props.onConnectPat}
      onCancelConnect={() => {
        void (async () => {
          await request('accounts.cancelConnect', {});
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
