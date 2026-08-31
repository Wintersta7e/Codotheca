import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  Collection,
  CollectionId,
  CommandArgs,
  CommandName,
  CommandResult,
} from '../../generated/protocol.js';
import type { CoreRpc } from './engine.js';
import {
  COLLECTIONS_LIST,
  COLLECTIONS_REMOVE,
  COLLECTIONS_UPSERT,
  CollectionsProvider,
  coreCollectionsApi,
  nextSortIndex,
  useCollections,
  type CollectionsApi,
} from './store.js';

afterEach(cleanup);

const saved = (id: number, sortIndex: number): Collection => ({
  id: id as CollectionId,
  name: `c${String(id)}`,
  kind: 'query',
  queryText: 'lang:rust',
  queryGrammarVersion: 1,
  sortIndex,
  memberCount: null,
});

function Probe(): ReactElement {
  const store = useCollections();
  return (
    <div>
      <span data-testid="status">{store.state.status}</span>
      <span data-testid="names">
        {store.state.status === 'ready' ? store.state.collections.map((c) => c.name).join(',') : ''}
      </span>
      <button
        type="button"
        onClick={() => {
          void store.remove(1 as CollectionId);
        }}
      >
        remove
      </button>
    </div>
  );
}

const mount = (api: CollectionsApi): void => {
  render(
    <CollectionsProvider api={api}>
      <Probe />
    </CollectionsProvider>,
  );
};

const statusText = (): string | null => screen.getByTestId('status').textContent;
const namesText = (): string | null => screen.getByTestId('names').textContent;

describe('nextSortIndex', () => {
  it('is max + 1, and 0 for an empty table', () => {
    expect(nextSortIndex([])).toBe(0);
    expect(nextSortIndex([saved(1, 0), saved(2, 7), saved(3, 3)])).toBe(8);
  });
});

describe('CollectionsProvider', () => {
  // Never render unknown as zero: before the reply there is no list, and `[]` would claim the
  // user has saved nothing.
  it('is loading before the first reply and ready after it', async () => {
    let settle: (rows: readonly Collection[]) => void = () => undefined;
    const api: CollectionsApi = {
      list: () =>
        new Promise((resolve) => {
          settle = resolve;
        }),
      create: vi.fn(),
      remove: vi.fn(),
    };
    mount(api);
    expect(statusText()).toBe('loading');
    settle([saved(1, 0)]);
    await waitFor(() => {
      expect(statusText()).toBe('ready');
    });
    expect(namesText()).toBe('c1');
  });

  it('is unavailable, not empty, when the request fails', async () => {
    mount({
      list: () => Promise.reject(new Error('CORE_UNAVAILABLE')),
      create: vi.fn(),
      remove: vi.fn(),
    });
    await waitFor(() => {
      expect(statusText()).toBe('unavailable');
    });
  });

  it('re-lists after a removal rather than patching its own array', async () => {
    const rows = [saved(1, 0), saved(2, 1)];
    const list = vi.fn(() => Promise.resolve(rows.slice()));
    const remove = vi.fn((id: CollectionId) => {
      const at = rows.findIndex((row) => row.id === id);
      rows.splice(at, 1);
      return Promise.resolve();
    });
    mount({ list, create: vi.fn(), remove });
    await waitFor(() => {
      expect(namesText()).toBe('c1,c2');
    });
    fireEvent.click(screen.getByRole('button', { name: 'remove' }));
    await waitFor(() => {
      expect(namesText()).toBe('c2');
    });
    expect(list).toHaveBeenCalledTimes(2);
  });
});

interface RpcProbe {
  readonly rpc: CoreRpc;
  readonly calls: { name: CommandName; args: unknown }[];
}

/**
 * A `CoreRpc` that records what it was asked and answers from the generated result types. The
 * schema now declares all three commands, so nothing here is cast through `never`.
 */
function fakeRpc(replies: Partial<CommandResult>): RpcProbe {
  const calls: { name: CommandName; args: unknown }[] = [];
  const rpc: CoreRpc = {
    request: <K extends CommandName>(name: K, args: CommandArgs[K]): Promise<CommandResult[K]> => {
      calls.push({ name, args });
      return Promise.resolve(replies[name] as CommandResult[K]);
    },
  };
  return { rpc, calls };
}

describe('coreCollectionsApi', () => {
  // [R37] `collections.list` returns a BARE `[Collection]` (`protocol.json:986`), not
  // `{ collections }`. Reading `reply.collections` would be `undefined` against every real
  // reply and the chip row would render empty forever, silently and without an error.
  it('unwraps the bare list the schema declares, not a wrapper object', async () => {
    const probe = fakeRpc({ 'collections.list': [saved(1, 0)] });
    await expect(coreCollectionsApi(probe.rpc).list()).resolves.toEqual([saved(1, 0)]);
    expect(probe.calls).toEqual([{ name: COLLECTIONS_LIST, args: {} }]);
  });

  it('names the three commands in one place and sends the upsert shape the schema declares', async () => {
    const probe = fakeRpc({
      'collections.upsert': { collection: saved(1, 4), refusedBecause: null },
    });
    const api = coreCollectionsApi(probe.rpc);

    await expect(
      api.create({
        name: 'Rust work',
        queryText: 'lang:rust',
        queryGrammarVersion: 1,
        sortIndex: 4,
      }),
    ).resolves.toBeNull();
    expect(probe.calls.at(-1)).toEqual({
      name: COLLECTIONS_UPSERT,
      // `id: null` is required by `CollectionsUpsertArgs` — omitting it is a create that the
      // core reads as an update of collection `undefined`.
      args: {
        id: null,
        name: 'Rust work',
        kind: 'query',
        queryText: 'lang:rust',
        queryGrammarVersion: 1,
        sortIndex: 4,
      },
    });

    await api.remove(9 as CollectionId);
    expect(probe.calls.at(-1)).toEqual({ name: COLLECTIONS_REMOVE, args: { id: 9 } });
  });

  // The core is the writer and has the last word — §1.9's `UNIQUE(name COLLATE NOCASE)` can
  // reject a name the renderer had already cleared. Dropping `refusedBecause` on the floor
  // makes that a silent no-op: the field closes and no chip appears.
  it('returns the refusal the core replied with rather than discarding it', async () => {
    const probe = fakeRpc({
      'collections.upsert': { collection: null, refusedBecause: 'name_taken' },
    });
    await expect(
      coreCollectionsApi(probe.rpc).create({
        name: 'Rust work',
        queryText: 'lang:rust',
        queryGrammarVersion: 1,
        sortIndex: 0,
      }),
    ).resolves.toBe('name_taken');
  });
});
