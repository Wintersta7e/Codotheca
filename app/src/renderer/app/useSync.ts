/**
 * §21's lane, as the renderer sees it: one topic, one status, one notice candidate.
 *
 * **The renderer computes nothing.** It holds the last `sync` payload and re-renders it — the
 * count, the denominator and the notice are all the core's answers. A figure derived here would
 * be a second owner for a value §21.11 gives exactly one.
 */
import { useEffect, useMemo, useState } from 'react';

import type { SyncListingProgress, SyncNotice, SyncStatus } from '../../generated/protocol.js';
import type { AppDeps } from './deps.js';

export interface SyncState {
  /** The last `sync/snapshot`, or `null` before one has arrived. Never a synthesised empty one. */
  readonly status: SyncStatus | null;
  /**
   * §21.11's line, or `null` when no listing is in flight. **Never a percentage**, and never a
   * figure that retreats.
   */
  readonly progressLine: string | null;
  /** §21.10's one banner variant, or `null` for *no sync failure* — never a default variant. */
  readonly notice: SyncNotice | null;
}

/**
 * §21.11, both clauses.
 *
 * - A total the **response** supplied renders `<n> of <total>`.
 * - No total renders a **bare count**. A denominator inferred from page numbers is a guess, and a
 *   guessed denominator is an unknown rendered as a fact.
 * - **Never a percentage.** There is no arithmetic here at all, which is what makes that
 *   structural rather than asserted.
 *
 * No forge listing in phase 2 supplies a total, so the first branch is unreachable against today's
 * core. It stays because the type has to be able to say *unknown*, and because the branch that
 * renders a real denominator must exist before one arrives rather than after.
 */
export function formatListingProgress(progress: SyncListingProgress): string {
  const listed = String(progress.listed);
  return progress.total === null ? listed : `${listed} of ${String(progress.total)}`;
}

/**
 * The `sync` topic, subscribed once through `AppDeps`' fan-out.
 *
 * **The count only ever increases**, and that is the core's property rather than this hook's:
 * `ListingProgress::add` saturates upward and has no setter, so no event can carry a lower figure
 * than the one before it. What this hook must not do is compute a figure of its own, and it does
 * not.
 */
export function useSync(deps: AppDeps): SyncState {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [listing, setListing] = useState<SyncListingProgress | null>(null);
  const [notice, setNotice] = useState<SyncNotice | null>(null);

  const { subscribe } = deps;

  useEffect(
    () =>
      subscribe((event) => {
        if (event.topic !== 'sync') return;
        if (event.event === 'snapshot') {
          const snapshot = event.data as SyncStatus;
          setStatus(snapshot);
          setListing(snapshot.listing);
          setNotice(snapshot.notice);
          return;
        }
        if (event.event === 'listing_progress') {
          setListing(event.data as SyncListingProgress);
          return;
        }
        if (event.event === 'notice') {
          setNotice(event.data as SyncNotice);
        }
      }),
    [subscribe],
  );

  return useMemo(
    () => ({
      status,
      progressLine: listing === null ? null : formatListingProgress(listing),
      notice,
    }),
    [status, listing, notice],
  );
}
