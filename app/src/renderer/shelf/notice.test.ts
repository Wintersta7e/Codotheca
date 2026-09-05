import { describe, expect, it } from 'vitest';
import type { Notice, NoticeKind } from './notice.js';
import {
  NOTICE_PRIORITY,
  noticeAccent,
  noticeDismissKey,
  noticeIsDismissible,
  selectNotice,
} from './notice.js';

const notice = (kind: NoticeKind, scope: string | null = null): Notice => ({
  kind,
  scope,
  title: `${kind} title`,
  body: `${kind} body`,
  actions: [],
});

describe('the priority order', () => {
  // [p2] §20.11 appends `connect` last. The transcription moves with the array rather than the
  // array being trimmed to keep this literal: a count that disagrees with the table moves the
  // failure instead of fixing it.
  it("is §8.0's table, in order, plus §20.11's offer last", () => {
    expect(NOTICE_PRIORITY).toEqual([
      'coreFailure',
      'scanResumed',
      'problems',
      'targetUnresolved',
      'identity',
      'residency',
      'newArrivals',
      'connect',
    ]);
  });
  it('carries no destructive kind', () => {
    // Phase 1 has no destructive operation at all.
    expect(NOTICE_PRIORITY.join(' ')).not.toMatch(/forget|delete|remove|uninstall/i);
  });
});

describe('selectNotice', () => {
  it('renders at most one — never two', () => {
    const chosen = selectNotice([notice('newArrivals', 'run-7'), notice('problems', 'run-7')], []);
    expect(chosen?.kind).toBe('problems');
  });
  it('picks by priority and not by the order the candidates arrive in', () => {
    // The mirror of the case above: swapping the array must not swap the answer.
    const chosen = selectNotice([notice('problems', 'run-7'), notice('newArrivals', 'run-7')], []);
    expect(chosen?.kind).toBe('problems');
  });
  it('returns null when nothing qualifies, so the wrapper is not rendered at all', () => {
    expect(selectNotice([], [])).toBeNull();
  });
  it('suppresses only the dismissed notice, never the slot', () => {
    const chosen = selectNotice(
      [notice('problems', 'run-7'), notice('newArrivals', 'run-7')],
      ['notice.dismissed.problems:run-7'],
    );
    expect(chosen?.kind).toBe('newArrivals');
  });
  it('scopes dismissal, so the next scan run raises the same kind again', () => {
    const chosen = selectNotice([notice('problems', 'run-8')], ['notice.dismissed.problems:run-7']);
    expect(chosen?.kind).toBe('problems');
  });
  it('stands the priority-1 notice even against its own dismissal key', () => {
    // It has no dismissal at all: it stands until the condition clears.
    const chosen = selectNotice([notice('coreFailure')], ['notice.dismissed.coreFailure']);
    expect(chosen?.kind).toBe('coreFailure');
    expect(noticeIsDismissible('coreFailure')).toBe(false);
  });
  it('ignores a candidate whose kind is unknown to the order', () => {
    expect(
      selectNotice([{ ...notice('problems'), kind: 'invented' as NoticeKind }], []),
    ).toBeNull();
  });
});

describe('noticeDismissKey', () => {
  it('scopes when there is a scope', () => {
    expect(noticeDismissKey('problems', 'run-7')).toBe('notice.dismissed.problems:run-7');
  });
  it('is unscoped when the ask is asked once, ever', () => {
    // §8.0 row 4a and §1.9 both spell this key `notice.dismissed.identity`. It is a stored
    // value, so the kind name is what the spec spells and not a synonym.
    expect(noticeDismissKey('identity', null)).toBe('notice.dismissed.identity');
  });
  it('names every kind with the spelling the stored key uses', () => {
    // One owner per value: the key is `notice.dismissed.` + the kind, with no translation
    // table in between, so nothing can drift from §1.9's stored form.
    for (const kind of NOTICE_PRIORITY) {
      expect(noticeDismissKey(kind, null)).toBe(`notice.dismissed.${kind}`);
      expect(noticeDismissKey(kind, '41')).toBe(`notice.dismissed.${kind}:41`);
    }
  });
});

describe('noticeAccent', () => {
  it('gives priority 1 the hot border and everything else the accent', () => {
    // Only the left border varies.
    expect(noticeAccent('coreFailure')).toBe('fail-hot');
    for (const kind of NOTICE_PRIORITY.slice(1)) expect(noticeAccent(kind)).toBe('sig');
  });
});

describe('[p2] §20.11 — the connect offer, exactly one and lowest', () => {
  const offer = (): Notice => ({
    kind: 'connect',
    // Unscoped: answering **or** dismissing ends it and it is never re-raised.
    scope: null,
    title: 'CONNECT GITHUB',
    body: 'Stars, issues, pull requests and CI state stay unknown until a token exists.',
    actions: [],
  });

  const arrivals = (scanRunId: string): Notice => ({
    kind: 'newArrivals',
    scope: scanRunId,
    title: 'NEW',
    body: 'new arrivals',
    actions: [],
  });

  it('sorts last, and its length grew rather than a row being replaced', () => {
    expect(NOTICE_PRIORITY.at(-1)).toBe('connect');
    expect(NOTICE_PRIORITY).toHaveLength(8);
    expect(new Set(NOTICE_PRIORITY).size).toBe(NOTICE_PRIORITY.length);
  });

  it('loses to every other candidate, which is what "exactly one" rests on', () => {
    for (const kind of NOTICE_PRIORITY) {
      if (kind === 'connect') continue;
      const other: Notice = { kind, scope: null, title: 'T', body: 'B', actions: [] };
      expect(selectNotice([offer(), other], [])?.kind).toBe(kind);
    }
  });

  it('is dismissible, and its key is unscoped', () => {
    expect(noticeIsDismissible('connect')).toBe(true);
    expect(noticeDismissKey('connect', null)).toBe('notice.dismissed.connect');
  });

  it('is never re-raised once dismissed, including after a new scan run', () => {
    const dismissed = [noticeDismissKey('connect', null)];
    expect(selectNotice([offer()], dismissed)).toBeNull();
    // Every other dismissible notice is scoped by the run id and comes back on the next scan.
    // This one must not: an unscoped key is what makes "never re-raised" true.
    expect(selectNotice([offer(), arrivals('run-2')], dismissed)?.kind).toBe('newArrivals');
    expect(selectNotice([offer()], dismissed)).toBeNull();
  });

  it('takes the accent every non-failure notice takes', () => {
    expect(noticeAccent('connect')).toBe('sig');
  });
});
