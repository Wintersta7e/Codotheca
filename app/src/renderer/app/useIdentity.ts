/**
 * §1.4's identity set, and the one surface that can correct it.
 *
 * The set is seeded by the core at startup and every figure on the shelf is computed from it:
 * `authored_by_user` and `is_reference` are a fold of each repository's committers against these
 * addresses, and §8.0b's bare query returns no Reference rows. So an address the user does not
 * recognise is not a detail — it decides which projects are on their shelf at all.
 *
 * `rows === null` is *not read*, never an empty set: a failed read that rendered zero rows would
 * raise §1.4's card over a set the app never saw and invite the user to confirm nothing.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import type { IdentityConfirm, IdentityRow } from '../../generated/protocol.js';
import type { RendererEvent } from '../../shared/channels.js';
import type { AppDeps } from './deps.js';
import { AUTHORSHIP_REREAD_MS, endsAScanRun, settlesAuthorship } from './useLibrary.js';

export interface IdentityState {
  /** `null` until `identity.list` answers. */
  readonly rows: readonly IdentityRow[] | null;
  /** §1.4: the effect precedes the write, so it is the `apply:false` preview and not the write. */
  readonly preview: IdentityConfirm | null;
  readonly askPreview: (emails: readonly string[]) => void;
  readonly confirm: (emails: readonly string[]) => void;
}

/**
 * Whether §8.0's row 4a has anything to ask.
 *
 * Unscoped and one-shot: once any row carries a `confirmedAt`, the set is the user's answer and
 * the card is over — a rule that lives in the index rather than in a dismissal key, so quitting
 * before answering does not lose the question and answering it does not ask again.
 */
export function identityNeedsConfirming(rows: readonly IdentityRow[] | null): boolean {
  return rows !== null && rows.length > 0 && rows.every((row) => row.confirmedAt === null);
}

export function useIdentity(deps: AppDeps, onApplied: () => void): IdentityState {
  const [rows, setRows] = useState<readonly IdentityRow[] | null>(null);
  const [preview, setPreview] = useState<IdentityConfirm | null>(null);
  const { request, subscribe } = deps;
  const live = useRef(true);
  // The reload is a seam, not a dependency: it is rebuilt by its owner on every library change,
  // and a subscription keyed on it would drop events in the gap between the two.
  const applied = useRef(onApplied);
  useEffect(() => {
    applied.current = onApplied;
  });

  const read = useCallback(() => {
    void request('identity.list', {}).then(
      (answer) => {
        if (live.current) setRows(answer);
      },
      () => {
        // Unread stays unread: §1.4's card is not raised over a set nothing could read.
      },
    );
  }, [request]);

  useEffect(() => {
    live.current = true;
    read();
    return () => {
      live.current = false;
    };
  }, [read]);

  // A scan's authorship jobs write `project_committer`, which is what the derived rules seed
  // from, so the set a finished run leaves is not the set the app read at mount — and the jobs
  // go on settling after the run has ended, so the end of the run is not the last word either.
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    const cancelPending = (): void => {
      if (pending.current !== null) clearTimeout(pending.current);
      pending.current = null;
    };
    const unsubscribe = subscribe((event: RendererEvent) => {
      if (endsAScanRun(event)) {
        // Issued after every settle that came before it, so a re-read one of them left pending
        // would only read the same set again.
        cancelPending();
        read();
      } else if (settlesAuthorship(event) && pending.current === null) {
        pending.current = setTimeout(() => {
          pending.current = null;
          read();
        }, AUTHORSHIP_REREAD_MS);
      }
    });
    return () => {
      unsubscribe();
      cancelPending();
    };
  }, [subscribe, read]);

  const askPreview = useCallback(
    (emails: readonly string[]) => {
      void request('identity.confirm', { emails: [...emails], apply: false }).then(
        (answer) => {
          if (live.current) setPreview(answer);
        },
        () => {
          // No preview is no effect line, which is §1.4's own "unchanged set" rendering.
        },
      );
    },
    [request],
  );

  const confirm = useCallback(
    (emails: readonly string[]) => {
      void request('identity.confirm', { emails: [...emails], apply: true }).then(
        () => {
          if (!live.current) return;
          // Authorship was just recomputed, so the shelf is holding rows computed from the old
          // set. Both reads are re-issued: the card's own state is what makes it stop asking.
          read();
          applied.current();
        },
        () => {
          // A refused write leaves the set as it was and the card as it was — asking again is
          // the honest outcome, and it is what the next launch does.
        },
      );
    },
    [request, read],
  );

  return { rows, preview, askPreview, confirm };
}
