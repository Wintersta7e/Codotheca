import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { Problems, ScanRunId } from '../../generated/protocol';
import { noticeDismissKey, selectNotice, type Notice } from '../shelf/notice';
import { useNotices, type NoticeInput } from './useNotices';

function problems(count: number | null, runId: number | null = 4): Problems {
  return {
    runId: runId as ScanRunId | null,
    header: {
      walkedDirs: 10,
      repositories: 3,
      problemCount: count,
      ambiguousLineageCount: 0,
    },
    groups: [],
  };
}

function input(over: Partial<NoticeInput> = {}): NoticeInput {
  return {
    degraded: null,
    gitVersion: null,
    spawnFailure: null,
    problems: null,
    identityToConfirm: false,
    sync: null,
    onOpenLog: vi.fn(),
    onOpenScanSummary: vi.fn(),
    ...over,
  };
}

const notices = (over: Partial<NoticeInput> = {}): readonly Notice[] =>
  renderHook(() => useNotices(input(over))).result.current;

describe('useNotices', () => {
  it('raises nothing when nothing is wrong', () => {
    expect(notices()).toEqual([]);
  });

  it('a degraded core is priority 1 and takes its words from §11.1', () => {
    const raised = notices({ degraded: 'git_too_old', gitVersion: '2.20.1' });
    expect(raised.map((n) => n.kind)).toEqual(['coreFailure']);
    expect(raised[0]?.title).toBe('THIS GIT IS TOO OLD');
    // The version it found, printed by the copy this hook consumes rather than restated here.
    expect(raised[0]?.body).toContain('2.20.1');
  });

  it('a dismissed coreFailure is still rendered', () => {
    // That is what UNDISMISSABLE means, and it is the assertion a naive filter fails: a hook
    // that filtered its own candidates by dismissedNotices would hide the one notice §8.0 says
    // stands until the condition clears.
    const raised = notices({ degraded: 'git_missing' });
    const dismissed = [noticeDismissKey('coreFailure', null)];
    expect(selectNotice(raised, dismissed)?.kind).toBe('coreFailure');
  });

  it('a dismissed problems notice is not rendered, because that one is dismissible', () => {
    const raised = notices({ problems: problems(3) });
    expect(raised.map((n) => n.kind)).toEqual(['problems']);
    const dismissed = [noticeDismissKey('problems', raised[0]?.scope ?? null)];
    expect(selectNotice(raised, dismissed)).toBeNull();
  });

  it('scopes the problems dismissal to the run, so the next scan asks again', () => {
    const first = notices({ problems: problems(3, 4) });
    const second = notices({ problems: problems(3, 5) });
    expect(first[0]?.scope).toBe('4');
    expect(second[0]?.scope).toBe('5');
    expect(selectNotice(second, [noticeDismissKey('problems', '4')])?.kind).toBe('problems');
  });

  it('a scan still counting raises no problems notice', () => {
    // `null` is a run in progress and `0` is a finished run with nothing to report. Neither is
    // a banner, and a null rendered as a banner would be unknown rendered as a fault.
    expect(notices({ problems: problems(null) })).toEqual([]);
    expect(notices({ problems: problems(0) })).toEqual([]);
  });

  it('the core lane wins over the scan when both have something to say', () => {
    const raised = notices({ degraded: 'index_read_only', problems: problems(3) });
    expect(selectNotice(raised, [])?.kind).toBe('coreFailure');
  });

  it('offers only actions that have somewhere to go', () => {
    const raised = notices({ degraded: 'git_missing' });
    const labels = raised[0]?.actions.map((action) => action.label) ?? [];
    // §11.3a forbids a control with nothing behind it. `OPEN THE LOG` reveals a named target
    // the shell already answers; `RETRY` has no command in the schema's forty-two, so it is
    // left out rather than drawn dead.
    expect(labels).toContain('OPEN THE LOG');
    expect(labels).not.toContain('RETRY');
  });

  it('runs the action it drew', () => {
    const onOpenLog = vi.fn();
    const raised = notices({ degraded: 'git_missing', onOpenLog });
    raised[0]?.actions.find((action) => action.label === 'OPEN THE LOG')?.run();
    expect(onOpenLog).toHaveBeenCalledTimes(1);
  });
});

/**
 * [p2] §21.10's banner, as a candidate.
 *
 * `null` is *no sync failure* and raises nothing; a variant raises **one** candidate, scoped to
 * that variant so its dismissal cannot hide a different failure later.
 */
describe("§21.10's sync candidate", () => {
  it('raises nothing when there is no sync failure', () => {
    const notices = renderHook(() => useNotices(input({ sync: null }))).result.current;
    expect(notices.filter((n) => n.kind === 'remoteSync')).toHaveLength(0);
  });

  it('raises exactly one candidate, scoped to the variant', () => {
    const notices = renderHook(() => useNotices(input({ sync: 'throttled' }))).result.current;
    const raised = notices.filter((n) => n.kind === 'remoteSync');
    expect(raised).toHaveLength(1);
    expect(raised[0]?.scope).toBe('throttled');
    expect(raised[0]?.title).toBe('THE FORGE IS RATE LIMITING THIS APP');
  });

  /**
   * **One banner, not one per failed task.** The hook takes a single variant and not a list, so
   * three failed tasks at once cannot become three candidates — the shape is what enforces it,
   * which is why this asserts the shape rather than counting a list that does not exist.
   */
  it('cannot raise more than one however many tasks failed', () => {
    for (const variant of ['throttled', 'unauthorized', 'forbidden', 'offline'] as const) {
      const notices = renderHook(() => useNotices(input({ sync: variant }))).result.current;
      expect(notices.filter((n) => n.kind === 'remoteSync')).toHaveLength(1);
    }
  });

  /** It loses to a scan's problems, which is §19.3's *"GitHub is additive, never a gate"*. */
  it('does not outrank a local problem', () => {
    const notices = renderHook(() => useNotices(input({ sync: 'offline', problems: problems(4) })))
      .result.current;
    expect(selectNotice(notices, [])?.kind).toBe('problems');
  });
});
