/**
 * `view.get` once, `ShelfView` in memory, and a debounced `view.set` of whatever changed.
 *
 * The patch is `shelf/viewState`'s `patchFor`, which returns `null` when nothing changed. That
 * return value is **acted on**: an idle shelf issues no `view.set` at all, and the test says so
 * rather than hoping. The comparison is against the last *persisted* view, not the last local
 * one, or a change and a change back would write a patch restating what is already stored.
 *
 * The debounce interval is handed in. A hook that read one from a clock of its own would be a
 * second owner of a value the mount already states.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { ViewState } from '../../generated/protocol.js';
import { DEFAULT_SHELF_VIEW, patchFor, viewFromState, type ShelfView } from '../shelf/viewState.js';
import type { AppDeps } from './deps.js';

export function useViewState(
  deps: AppDeps,
  debounceMs: number,
): [ShelfView, (next: ShelfView) => void] {
  const [view, setView] = useState<ShelfView>(DEFAULT_SHELF_VIEW);
  /** The last view the core has been told about. */
  const persisted = useRef<ShelfView>(DEFAULT_SHELF_VIEW);
  const pending = useRef<ShelfView | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const live = useRef(true);
  const { request } = deps;

  const flush = useCallback(() => {
    timer.current = null;
    const next = pending.current;
    pending.current = null;
    if (next === null) return;
    const patch = patchFor(persisted.current, next);
    if (patch === null) return;
    persisted.current = next;
    void request('view.set', { patch }).then(
      () => undefined,
      () => {
        // A refused write is not a reason to re-send on the next keystroke: the renderer's copy
        // is what the user is looking at, and the next real change carries the difference.
      },
    );
  }, [request]);

  useEffect(() => {
    live.current = true;
    void request('view.get', {}).then(
      (state: ViewState) => {
        if (!live.current) return;
        const restored = viewFromState(state);
        persisted.current = restored;
        setView(restored);
      },
      () => {
        // §8's default view. A shelf that cannot read its stored view still opens.
      },
    );
    return () => {
      live.current = false;
      if (timer.current !== null) clearTimeout(timer.current);
    };
  }, [request]);

  const change = useCallback(
    (next: ShelfView) => {
      setView(next);
      pending.current = next;
      if (timer.current !== null) clearTimeout(timer.current);
      timer.current = setTimeout(flush, debounceMs);
    },
    [flush, debounceMs],
  );

  return [view, change];
}
