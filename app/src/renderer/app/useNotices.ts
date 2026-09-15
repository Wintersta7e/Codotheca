/**
 * §8.0's one notice slot: the candidate list, and nothing else.
 *
 * `shelf/notice.ts` already declares the kinds, the priority order, the undismissable set and
 * `selectNotice`. This hook builds candidates and hands them over — it does **not** filter by
 * `dismissedNotices`, because a dismissed `coreFailure` is still rendered and a filter here
 * would silently overrule `UNDISMISSABLE`.
 *
 * A notice with no source is not rendered. §8.0 has six obligations and this raises the two
 * that have a producer today; a third invented here would be inventing a surface.
 */
import { useMemo } from 'react';

import type { DegradedReason, Problems } from '../../generated/protocol.js';
import type { NoticeCopy, SpawnFailureFacts } from '../notices/copy.js';
import { IDENTITY_BODY_1, IDENTITY_TITLE } from '../firstrun/copy.js';
import { degradedNotice, problemsNotice, spawnFailureNotice } from '../notices/copy.js';
import type { Notice, NoticeAction, NoticeKind } from '../shelf/notice.js';

export interface NoticeInput {
  /** §8.0 priority 1. `null` is *not degraded*. */
  readonly degraded: DegradedReason | null;
  readonly gitVersion: string | null;
  /**
   * §11.2's five spawn-failure sentences, which only the main process can tell apart — it is
   * the only side that can stat the binary. `null` until plan 19's `coreFailure.ts` supplies
   * them; this hook authors none of them and raises nothing without them.
   */
  readonly spawnFailure: SpawnFailureFacts | null;
  /** §11.1's problem count. `null` is a scan that has not reported one. */
  readonly problems: Problems | null;
  /**
   * §8.0's row 4a. True when §1.4's card has something to ask — the set is seeded and nothing in
   * it has been confirmed. The card draws its own contents through the slot's `renderContent`;
   * what belongs here is only whether the row qualifies at all.
   */
  readonly identityToConfirm: boolean;
  readonly onOpenLog: () => void;
  readonly onOpenScanSummary: () => void;
}

/**
 * The copy names a primary and a secondary; this draws only the ones with a producer.
 *
 * §11.3a forbids a control with nothing behind it, and it is the same rule here: `RETRY` on a
 * degraded core would have to re-run the git floor check, which the core reads once at startup
 * and which no command in the schema's forty-two re-runs. Drawing it dead is worse than not
 * drawing it, so it is left out and recorded.
 */
function actionsFor(
  copy: NoticeCopy,
  runners: Readonly<Record<string, (() => void) | undefined>>,
): readonly NoticeAction[] {
  const out: NoticeAction[] = [];
  const primary = copy.primary === null ? undefined : runners[copy.primary];
  if (copy.primary !== null && primary !== undefined) {
    out.push({ label: copy.primary, kind: 'primary', run: primary });
  }
  const secondary = copy.secondary === null ? undefined : runners[copy.secondary];
  if (copy.secondary !== null && secondary !== undefined) {
    out.push({ label: copy.secondary, kind: 'secondary', run: secondary });
  }
  return out;
}

function toNotice(
  kind: NoticeKind,
  scope: string | null,
  copy: NoticeCopy,
  runners: Readonly<Record<string, (() => void) | undefined>>,
): Notice {
  return {
    kind,
    scope,
    title: copy.title,
    // §11.3's measured residency line and §11.2a's log path both arrive on `note`, which the
    // slot renders as part of the body rather than as a second block it does not have.
    body: copy.note === null ? copy.body : `${copy.body} ${copy.note}`,
    actions: actionsFor(copy, runners),
  };
}

export function useNotices(input: NoticeInput): readonly Notice[] {
  const {
    degraded,
    gitVersion,
    spawnFailure,
    problems,
    identityToConfirm,
    onOpenLog,
    onOpenScanSummary,
  } = input;

  return useMemo(() => {
    const out: Notice[] = [];
    const runners: Record<string, (() => void) | undefined> = {
      'OPEN THE LOG': onOpenLog,
      'SEE THE SUMMARY': onOpenScanSummary,
    };

    const lane =
      spawnFailure !== null
        ? spawnFailureNotice(spawnFailure)
        : degraded === null
          ? null
          : degradedNotice(degraded, gitVersion);
    if (lane !== null) out.push(toNotice('coreFailure', null, lane, runners));

    const scan = problems === null ? null : problemsNotice(problems);
    if (scan !== null && problems !== null) {
      // Scoped to the run, so dismissing one scan's problems does not silence the next one's.
      out.push(toNotice('problems', String(problems.runId), scan, runners));
    }

    if (identityToConfirm) {
      // Unscoped: §1.4's card is one-shot and belongs to the library, not to a run. The title
      // is what the slot uses as its accessible name; the body and every action are the card's,
      // supplied through `renderContent`.
      out.push({
        kind: 'identity',
        scope: null,
        title: IDENTITY_TITLE,
        body: IDENTITY_BODY_1,
        actions: [],
      });
    }

    return out;
  }, [
    degraded,
    gitVersion,
    spawnFailure,
    problems,
    identityToConfirm,
    onOpenLog,
    onOpenScanSummary,
  ]);
}
