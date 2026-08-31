import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react';
import type { Collection, CollectionId, CollectionRefusal } from '../../generated/protocol.js';
import type { CoreRpc } from './engine.js';

/**
 * The three command names §2.4 ships for collections, in one place. The schema declares all
 * three now (`protocol/schema/protocol.json:986-1003`), so the calls below are typed end to end
 * and this module is the only place the argument shape is stated.
 */
export const COLLECTIONS_LIST = 'collections.list';
export const COLLECTIONS_UPSERT = 'collections.upsert';
export const COLLECTIONS_REMOVE = 'collections.remove';

export interface CollectionDraft {
  readonly name: string;
  readonly queryText: string;
  readonly queryGrammarVersion: number;
  readonly sortIndex: number;
}

export interface CollectionsApi {
  list(): Promise<readonly Collection[]>;
  /**
   * `null` when the core stored it. The core is the writer and refuses again — §1.9's
   * `UNIQUE(name COLLATE NOCASE)` can reject a name this renderer had already cleared — so the
   * reply's `refusedBecause` is carried out rather than dropped. Discarding it would turn a
   * refused save into a silent no-op: the field closes and no chip ever appears.
   */
  create(draft: CollectionDraft): Promise<CollectionRefusal | null>;
  remove(id: CollectionId): Promise<void>;
}

/** §8.8: phase 1 renders both kinds and creates only the query kind. */
export function coreCollectionsApi(rpc: CoreRpc): CollectionsApi {
  return {
    async list() {
      // [R37] `collections.list` returns a BARE `[Collection]`, not `{ collections }`. A
      // `reply.collections ?? []` would be `undefined ?? []` against every real reply and the
      // chip row would render empty forever, silently and without an error.
      return await rpc.request(COLLECTIONS_LIST, {});
    },
    async create(draft) {
      const reply = await rpc.request(COLLECTIONS_UPSERT, {
        // `CollectionsUpsertArgs.id` is `CollectionId | null` and the field is required:
        // creating is `id: null`, and omitting it is not the same message.
        id: null,
        name: draft.name,
        kind: 'query',
        queryText: draft.queryText,
        queryGrammarVersion: draft.queryGrammarVersion,
        sortIndex: draft.sortIndex,
      });
      return reply.refusedBecause;
    },
    async remove(id) {
      await rpc.request(COLLECTIONS_REMOVE, { id });
    },
  };
}

export type CollectionsState =
  | { readonly status: 'loading' }
  | { readonly status: 'ready'; readonly collections: readonly Collection[] }
  | { readonly status: 'unavailable' };

/** §8.8: `collections.upsert` with `sort_index = max(sort_index) + 1`. */
export function nextSortIndex(collections: readonly Collection[]): number {
  return collections.reduce((max, c) => Math.max(max, c.sortIndex + 1), 0);
}

export interface CollectionsStore {
  readonly state: CollectionsState;
  readonly reload: () => void;
  readonly create: (draft: CollectionDraft) => Promise<CollectionRefusal | null>;
  readonly remove: (id: CollectionId) => Promise<void>;
}

const CollectionsContext = createContext<CollectionsStore | null>(null);

export interface CollectionsProviderProps {
  readonly api: CollectionsApi;
  readonly children: ReactNode;
}

export function CollectionsProvider(props: CollectionsProviderProps): ReactElement {
  const [state, setState] = useState<CollectionsState>({ status: 'loading' });
  const api = props.api;
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  const reloadNow = useCallback(async (): Promise<void> => {
    try {
      const collections = await api.list();
      if (live.current) setState({ status: 'ready', collections });
    } catch {
      // §2.4: the core's `message` is diagnostic and is never shown raw. The row renders
      // nothing, which is the absence of a claim rather than a claim of emptiness.
      if (live.current) setState({ status: 'unavailable' });
    }
  }, [api]);

  useEffect(() => {
    void reloadNow();
  }, [reloadNow]);

  const store = useMemo<CollectionsStore>(
    () => ({
      state,
      reload: () => {
        void reloadNow();
      },
      // Every mutation awaits the core and then re-lists rather than patching a local array: a
      // locally-patched row would be the renderer's guess at what the core wrote.
      create: async (draft) => {
        const refusal = await api.create(draft);
        await reloadNow();
        return refusal;
      },
      // §8.8: if the deleted collection was active the shelf's query is left exactly as it is.
      // Nothing here touches the query, and nothing here touches a project.
      remove: async (id) => {
        await api.remove(id);
        await reloadNow();
      },
    }),
    [api, reloadNow, state],
  );

  return createElement(CollectionsContext.Provider, { value: store }, props.children);
}

export function useCollections(): CollectionsStore {
  const store = useContext(CollectionsContext);
  if (store === null) throw new Error('useCollections outside CollectionsProvider');
  return store;
}
