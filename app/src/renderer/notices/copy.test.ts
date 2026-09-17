import { describe, expect, it } from 'vitest';
import type { Problems, SyncNotice, TargetVerification } from '../../generated/protocol';
import { GIT_FLOOR } from '../../shared/gitFloor';
import {
  degradedNotice,
  GIT_FLOOR_DISPLAY,
  problemsNotice,
  remoteSyncNotice,
  residencyNotice,
  RESIDENCY_MEASUREMENTS,
  spawnFailureNotice,
  staleTargetsNotice,
} from './copy';

function problems(patch: Record<string, unknown> = {}): Problems {
  const { runId = 1, ...header } = patch;
  return {
    runId,
    header: {
      walkedDirs: 1000,
      repositories: 40,
      problemCount: 9,
      ambiguousLineageCount: 2,
      ...header,
    },
    groups: [],
  } as unknown as Problems;
}

function target(patch: Record<string, unknown> = {}): TargetVerification {
  return {
    targetId: 1,
    verifyState: 'missing',
    verifiedAt: 100,
    execDisplay: 'editor.exe',
    ...patch,
  } as unknown as TargetVerification;
}

describe('the git floor is one value, not two', () => {
  // `shared/gitFloor.ts` already mirrors `core/src/git/version.rs` and `gitFloor.test.ts`
  // already reads the Rust source to keep them equal (R24). A second literal here would be a
  // third copy with nothing holding it to the other two.
  it('is displayed from the mirror the core is tested against', () => {
    expect(GIT_FLOOR_DISPLAY).toBe(GIT_FLOOR.join('.'));
    expect(GIT_FLOOR_DISPLAY).toBe('2.22');
  });
});

describe('priority 1: the core is not alive', () => {
  it('names the floor when git is missing', () => {
    const n = degradedNotice('git_missing', null);
    expect(n?.body).toContain(GIT_FLOOR_DISPLAY);
    expect(n?.primary).toBe('RETRY');
    expect(n?.secondary).toBe('OPEN THE LOG');
  });

  it('names both the floor and the version found when git is too old', () => {
    const n = degradedNotice('git_too_old', '2.17.1');
    expect(n?.body).toContain('2.22');
    expect(n?.body).toContain('2.17.1');
  });

  it('never renders a missing version as a value', () => {
    const body = degradedNotice('git_too_old', null)?.body ?? '';
    expect(body).toContain('2.22');
    expect(body).not.toContain('null');
    expect(body).not.toMatch(/git 0|version 0/);
  });

  it('raises nothing for the two per-project degradations', () => {
    expect(degradedNotice('store_offline', null)).toBeNull();
    expect(degradedNotice('budget_exceeded', null)).toBeNull();
  });

  it('raises one for a read-only index, which no other surface reports', () => {
    expect(degradedNotice('index_read_only', null)?.title).toMatch(/saved|read-only|saving/i);
  });

  it('adapts the shell’s spawn sentence rather than writing a second one', () => {
    const fromShell = {
      title: 'CODOTHECA COULD NOT START ITS CORE',
      body: 'The core binary is on a filesystem mounted noexec.',
      logPath: '/data/codotheca.log',
      primary: 'RETRY',
      secondary: 'OPEN THE LOG',
    };
    const n = spawnFailureNotice(fromShell);
    expect(n.title).toBe(fromShell.title);
    expect(n.body).toBe(fromShell.body);
    expect(n.note).toContain('/data/codotheca.log');
    expect(n.primary).toBe('RETRY');
  });
});

describe('priority 3: problems from the last run', () => {
  it('reports a finished run that found some, with the count in the headline', () => {
    const n = problemsNotice(problems());
    // The plan's own test asserted the count in `body` while its implementation put it in
    // `title` (17c:2228 against 17c:2381). The headline is where it belongs — a notice is read
    // by its first line — so the assertion moves rather than the string.
    expect(n?.title).toContain('9');
    expect(n?.primary).toBe('SEE THE SUMMARY');
  });

  it('says nothing while a scan is in flight — null is not zero', () => {
    expect(problemsNotice(problems({ problemCount: null }))).toBeNull();
  });

  it('says nothing when a finished scan found none', () => {
    expect(problemsNotice(problems({ problemCount: 0 }))).toBeNull();
  });

  it('does not raise a notice for ambiguous lineage alone — nothing broke', () => {
    expect(problemsNotice(problems({ problemCount: 0, ambiguousLineageCount: 3 }))).toBeNull();
  });

  it('says nothing when no scan has run', () => {
    expect(problemsNotice(problems({ runId: null }))).toBeNull();
  });

  it('says one problem in the singular', () => {
    expect(problemsNotice(problems({ problemCount: 1 }))?.title).toContain('ONE PROBLEM');
  });
});

describe('priority 4: a launch target no longer resolves', () => {
  it('names what was tried, and offers re-detection', () => {
    const n = staleTargetsNotice([target()]);
    expect(n?.body).toContain('editor.exe');
    expect(n?.primary).toBe('RE-DETECT');
  });

  it('counts rather than lists when more than one has gone', () => {
    const n = staleTargetsNotice([
      target(),
      target({ targetId: 2, verifyState: 'not_executable' }),
    ]);
    expect(n?.title).toContain('2');
  });

  it('treats unverified as unknown, never as broken', () => {
    expect(staleTargetsNotice([target({ verifyState: 'unverified' })])).toBeNull();
    expect(staleTargetsNotice([target({ verifyState: 'ok' })])).toBeNull();
  });
});

describe('priority 5: the residency ask', () => {
  it('carries the measured numbers and not the figure that was false by 17x', () => {
    expect(RESIDENCY_MEASUREMENTS).toBe(
      '307 MB EMPTY · 522 MB WITH A FULL SHELF · 232 MB WITH THE WINDOW DESTROYED',
    );
    const n = residencyNotice();
    expect(n.note).toBe(RESIDENCY_MEASUREMENTS);
    expect(`${n.title} ${n.body}`).not.toContain('30 MB');
    expect(n.primary).toBeTruthy();
    expect(n.secondary).toBeTruthy();
  });

  // §11.3: the residency ask is "asked once, after value has been demonstrated, and never
  // again". §1.4 rules on the same shape for the identity card: a secondary that promises a
  // later ask "is a lie told in two words". The row stays reachable in settings, and the body
  // says so — the notice itself does not come back.
  it('does not promise a later ask it will never make', () => {
    const n = residencyNotice();
    expect(n.secondary).not.toMatch(/not now|later|remind/i);
    expect(n.body).toMatch(/settings/i);
  });
});

/**
 * §21.10's four sentences.
 *
 * **Exhaustive by type**: the array below is the generated enum's whole vocabulary, and a fifth
 * variant added to the schema fails `remoteSyncNotice`'s `never` arm at type-check rather than
 * falling through to a sentence written for something else.
 */
describe('remoteSyncNotice', () => {
  const ALL: readonly SyncNotice[] = ['throttled', 'unauthorized', 'forbidden', 'offline'];

  it('gives each of the four variants its own sentence', () => {
    const titles = ALL.map((kind) => remoteSyncNotice(kind).title);
    const bodies = ALL.map((kind) => remoteSyncNotice(kind).body);
    expect(new Set(titles).size).toBe(ALL.length);
    expect(new Set(bodies).size).toBe(ALL.length);
  });

  /**
   * §21.10: every affected remote field is **unknown — never `failed`, never zero**, and the
   * glyph is §8.4.1's `—`, which belongs to the field and not to a banner about the lane. A
   * banner that printed `0` or `failed` would be the invariant broken in the one place the user
   * is looking when something has gone wrong.
   */
  it('never says failed, never prints a zero, and draws no glyph of its own', () => {
    for (const kind of ALL) {
      const copy = remoteSyncNotice(kind);
      const text = `${copy.title} ${copy.body} ${copy.note ?? ''}`;
      expect(text.toLowerCase()).not.toContain('failed');
      expect(text).not.toMatch(/\b0\b/);
      expect(text).not.toContain('—');
    }
  });

  /** A control with nothing behind it is §11.3a's forbidden shape, so only the two that lead
   *  somewhere carry one — and neither offers a RETRY the user cannot make happen. */
  it('offers an action only where one exists', () => {
    expect(remoteSyncNotice('throttled').primary).toBeNull();
    expect(remoteSyncNotice('offline').primary).toBeNull();
    expect(remoteSyncNotice('unauthorized').primary).toBe('OPEN ACCOUNTS');
    expect(remoteSyncNotice('forbidden').primary).toBe('OPEN ACCOUNTS');
    for (const kind of ALL) {
      expect(remoteSyncNotice(kind).secondary).toBeNull();
    }
  });

  /** §21.10 forbids a blocking dialog and a score change; the copy must not imply either. */
  it('promises no retry the user has to make and threatens no score', () => {
    for (const kind of ALL) {
      const text = `${remoteSyncNotice(kind).title} ${remoteSyncNotice(kind).body}`.toLowerCase();
      expect(text).not.toContain('score');
      expect(text).not.toContain('xp');
    }
  });
});
