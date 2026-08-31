import type { ReactElement } from 'react';
import type { Collection, CollectionId, ProjectRow } from '../../generated/protocol.js';
import { CollectionChip } from './CollectionChip.js';
import { useArmedDelete } from './armedDelete.js';
import { collectionChipModel, isCollectionActive } from './chipModel.js';
import type { QueryEngine } from './engine.js';
import { useCollections } from './store.js';

/** §8.8: appended after the four built-ins in `sort_index` order. */
export function savedChipOrder(collections: readonly Collection[]): readonly Collection[] {
  return [...collections].sort((a, b) => a.sortIndex - b.sortIndex);
}

export interface CollectionChipRowProps {
  /**
   * `null` means the projection has not hydrated. A count is `engine.filter(ast, rows).length`,
   * and run against an empty array it returns `0` — a measured zero that was never measured
   * (§8.8). An empty array cannot say "not yet", so the type refuses to let one stand in.
   */
  readonly rows: readonly ProjectRow[] | null;
  readonly currentQuery: string;
  readonly engine: QueryEngine;
  /** R3: time through deps, never `Date.now()`. */
  readonly nowMs: () => number;
  readonly onQueryChange: (query: string) => void;
}

/**
 * The saved tail of §8.0b's row. The row itself, its wrap and the four built-ins that precede
 * these are plan 13c's `AttentionRow`; this renders chips and nothing else.
 *
 * §5.6 confines the dry one-line note to an opened project page: nothing here renders one.
 */
export function CollectionChipRow(props: CollectionChipRowProps): ReactElement | null {
  const store = useCollections();
  const armed = useArmedDelete({
    nowMs: props.nowMs,
    setTimer: (cb, ms) => globalThis.setTimeout(cb, ms),
    clearTimer: (handle) => {
      globalThis.clearTimeout(handle);
    },
    onCommit: (id: CollectionId) => {
      // §8.8: the shelf's query is left exactly as it is, active or not — the rows on screen
      // were never the collection's property, and clearing them would make deleting a name look
      // like deleting work.
      void store.remove(id);
    },
  });

  const rows = props.rows;
  // Nothing is not a chip showing zero and not a chip showing a blank where a number belongs.
  // It is the absence of the claim, which is the only honest reading before the measurement.
  if (rows === null || store.state.status !== 'ready') return null;

  return (
    <>
      {savedChipOrder(store.state.collections).map((collection) => {
        const model = collectionChipModel(collection, rows, props.engine);
        return (
          <CollectionChip
            key={String(collection.id)}
            model={model}
            active={isCollectionActive(model, props.currentQuery, props.engine)}
            armed={armed.armedId === collection.id}
            onActivate={props.onQueryChange}
            onPressRemove={armed.press}
            onDisarm={armed.disarm}
          />
        );
      })}
    </>
  );
}
