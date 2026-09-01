import { describe, expect, it } from 'vitest';
import {
  RESIDENCY_ANSWERS,
  firstRunNoticeScope,
  firstRunNotices,
  residencyAutostart,
  type FirstRunNoticeInput,
} from './notices';
import { NOTICE_PRIORITY, noticeDismissKey } from '../shelf/notice';

function input(over: Partial<FirstRunNoticeInput> = {}): FirstRunNoticeInput {
  return {
    firstRunCompletedAt: null,
    identityCount: 0,
    identityConfirmedAt: null,
    newArrivalCount: 0,
    scanRunId: 41,
    dismissed: [],
    ...over,
  };
}

const STAMPED = 1_760_000_000;

describe('§8.0: which first-run notice may have the one slot', () => {
  // §1.4: answering identity changes numbers the user was shown ninety seconds ago; answering
  // residency changes nothing they can see. At most one renders, so identity goes first.
  it('puts identity ahead of residency when both qualify', () => {
    expect(firstRunNotices(input({ identityCount: 3 }))).toEqual(['identity', 'residency']);
  });

  // R12/R18's rule applied to an order: §8.0's priority table is `shelf/notice.ts`'s and this
  // module sorts by it rather than carrying a second copy that could disagree.
  it('orders by §8.0s own priority table rather than a second copy of it', () => {
    const notices = firstRunNotices(
      input({ identityCount: 2, newArrivalCount: 3, firstRunCompletedAt: null }),
    );
    const positions = notices.map((kind) => NOTICE_PRIORITY.indexOf(kind));
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
    expect(NOTICE_PRIORITY.indexOf('identity')).toBeLessThan(NOTICE_PRIORITY.indexOf('residency'));
    expect(NOTICE_PRIORITY.indexOf('residency')).toBeLessThan(
      NOTICE_PRIORITY.indexOf('newArrivals'),
    );
  });

  // §1.4: the card must be dismissible without being answered, or the stamp behind it is
  // unreachable and §10.5a's NEW predicate never arms.
  it('hands the slot to residency once identity is dismissed', () => {
    const dismissed = [noticeDismissKey('identity', null)];
    expect(firstRunNotices(input({ identityCount: 3, dismissed }))).toEqual(['residency']);
  });

  // §1.4: `confirmed_at` is the durable record that the set was answered. Answering ends the
  // notice exactly as dismissing does, and neither is ever re-raised.
  it('retires identity on an answer as well as on a dismissal', () => {
    expect(firstRunNotices(input({ identityCount: 3, identityConfirmedAt: STAMPED }))).toEqual([
      'residency',
    ]);
  });

  it('does not ask about an empty identity set', () => {
    expect(firstRunNotices(input({ identityCount: 0 }))).toEqual(['residency']);
  });

  // §11.3a: the stamp is what the residency answer writes, so the stamp is what retires the row.
  it('retires the residency ask for good once first run is stamped', () => {
    expect(firstRunNotices(input({ firstRunCompletedAt: STAMPED }))).toEqual([]);
  });

  // §8.0 row 6: new arrivals never displaces a question.
  it('never lets new arrivals outrank a question', () => {
    expect(
      firstRunNotices(
        input({ identityCount: 2, newArrivalCount: 3, firstRunCompletedAt: STAMPED }),
      ),
    ).toEqual(['identity', 'newArrivals']);
  });

  it('scopes an arrivals dismissal to its run, so a later run raises the row again', () => {
    const dismissed = [noticeDismissKey('newArrivals', '41')];
    const base = { newArrivalCount: 3, firstRunCompletedAt: STAMPED, dismissed };
    expect(firstRunNotices(input({ ...base, scanRunId: 41 }))).toEqual([]);
    expect(firstRunNotices(input({ ...base, scanRunId: 42 }))).toEqual(['newArrivals']);
  });

  it('renders nothing when nothing qualifies', () => {
    expect(firstRunNotices(input({ firstRunCompletedAt: STAMPED }))).toEqual([]);
  });
});

describe('the dismissal scopes §8.0 gives these three', () => {
  it('leaves identity and residency unscoped and scopes arrivals to the run', () => {
    expect(firstRunNoticeScope('identity', 41)).toBeNull();
    expect(firstRunNoticeScope('residency', 41)).toBeNull();
    expect(firstRunNoticeScope('newArrivals', 41)).toBe('41');
    expect(firstRunNoticeScope('newArrivals', null)).toBeNull();
  });

  // The key formatter is `shelf/notice.ts`'s. This module supplies the scope and nothing else,
  // so a first-run dismissal and a shelf dismissal cannot spell the same key two ways.
  it('spells its keys through the one formatter', () => {
    expect(noticeDismissKey('identity', firstRunNoticeScope('identity', 41))).toBe(
      'notice.dismissed.identity',
    );
    expect(noticeDismissKey('newArrivals', firstRunNoticeScope('newArrivals', 41))).toBe(
      'notice.dismissed.newArrivals:41',
    );
  });
});

// §11.3a: `first_run_completed_at` is stamped when the row is **answered or dismissed**, and the
// stamp is the durable record. So a dismissal must send the same `autostart` patch an answer
// does. A bare `view_state` key would suppress the row with the stamp still NULL — and then
// `isNewArrival` is false for every project forever, silently.
describe('§11.3a: every way out of the residency row is an answer', () => {
  it('turns all three actions into an autostart value the patch can carry', () => {
    expect(residencyAutostart('startWithTheSystem')).toBe(true);
    expect(residencyAutostart('leaveItOff')).toBe(false);
    expect(residencyAutostart('dismissed')).toBe(false);
  });

  it('leaves no way out that sends nothing', () => {
    expect(RESIDENCY_ANSWERS).toEqual(['startWithTheSystem', 'leaveItOff', 'dismissed']);
    for (const action of RESIDENCY_ANSWERS) {
      expect(typeof residencyAutostart(action)).toBe('boolean');
    }
  });
});
