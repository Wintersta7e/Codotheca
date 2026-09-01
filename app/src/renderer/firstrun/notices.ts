/**
 * The three notices first run leaves on the shelf, and what qualifies each of them.
 *
 * These are **content for §8.0's one slot**, which `shelf/notice.ts` owns. Nothing here draws a
 * box, nothing here sorts by a private priority table and nothing here spells a dismissal key:
 * §8.0 says "geometry lives here and only here", and the same argument covers the order and the
 * key. This module decides only *whether* each of the three has anything to say.
 */
import { NOTICE_PRIORITY, noticeDismissKey } from '../shelf/notice';
import type { NoticeKind } from '../shelf/notice';

/** The three of §8.0's seven rows that first run is responsible for. */
export type FirstRunNoticeKind = Extract<NoticeKind, 'identity' | 'residency' | 'newArrivals'>;

const FIRST_RUN_KINDS: readonly FirstRunNoticeKind[] = ['identity', 'residency', 'newArrivals'];

export interface FirstRunNoticeInput {
  /** §1.9's stamp. Null means the residency ask is still owed. */
  readonly firstRunCompletedAt: number | null;
  /** How many addresses the scan seeded. Zero means there is nothing to ask about. */
  readonly identityCount: number;
  /**
   * §1.4's `confirmed_at`. Non-null is the durable record that the set was answered, and
   * answering ends this notice exactly as dismissing it does.
   */
  readonly identityConfirmedAt: number | null;
  readonly newArrivalCount: number;
  readonly scanRunId: number | null;
  /** The `notice.dismissed.*` keys already in `view_state`. */
  readonly dismissed: readonly string[];
}

/**
 * The scope a dismissal is recorded against, for `noticeDismissKey`.
 *
 * Identity is **unscoped**: §1.4's notice is library-wide and one-shot, so a dismissal keyed to
 * the run that raised it would let the next scan raise it again. Residency is unscoped for the
 * same reason. Arrivals are per `scan_run.id`, so a later run's batch is a new row.
 */
export function firstRunNoticeScope(
  kind: FirstRunNoticeKind,
  scanRunId: number | null,
): string | null {
  if (kind !== 'newArrivals') return null;
  return scanRunId === null ? null : String(scanRunId);
}

function qualifies(kind: FirstRunNoticeKind, input: FirstRunNoticeInput): boolean {
  switch (kind) {
    case 'identity':
      // §1.4: asked once. `confirmed_at` records the answer; the dismissal key below records
      // the refusal to answer. Either ends it, and neither is ever re-raised.
      return input.identityCount >= 1 && input.identityConfirmedAt === null;
    case 'residency':
      // §11.3a: the stamp is what the answer writes, so the stamp is what retires the row.
      return input.firstRunCompletedAt === null;
    case 'newArrivals':
      return input.newArrivalCount >= 1;
  }
}

/**
 * Every first-run notice that may occupy the slot, in §8.0's order.
 *
 * The caller merges this with the shelf's own candidates and lets `selectNotice` take the head;
 * returning the whole list rather than one notice keeps that merge a single sort.
 */
export function firstRunNotices(input: FirstRunNoticeInput): readonly FirstRunNoticeKind[] {
  return FIRST_RUN_KINDS.filter((kind) => {
    if (!qualifies(kind, input)) return false;
    const key = noticeDismissKey(kind, firstRunNoticeScope(kind, input.scanRunId));
    return !input.dismissed.includes(key);
  }).sort((a, b) => NOTICE_PRIORITY.indexOf(a) - NOTICE_PRIORITY.indexOf(b));
}

/** Every way out of §11.3a's row. There is no fourth, and none of them is silent. */
export const RESIDENCY_ANSWERS = ['startWithTheSystem', 'leaveItOff', 'dismissed'] as const;

export type ResidencyAnswer = (typeof RESIDENCY_ANSWERS)[number];

/**
 * The `autostart` value the `settings.set` patch must carry for a given way out.
 *
 * §11.3a stamps `first_run_completed_at` when the row is **answered or dismissed**, and the core
 * keys that stamp on the patch carrying `autostart` at all — so a dismissal sends the same patch
 * a `LEAVE IT OFF` answer does. Suppressing the row with a bare `view_state` key instead would
 * leave the stamp NULL, and then `isNewArrival` is false for every project forever: the `NEW`
 * chip and the arrivals row never appear at all. It does not fail; it never shows.
 */
export function residencyAutostart(answer: ResidencyAnswer): boolean {
  return answer === 'startWithTheSystem';
}
