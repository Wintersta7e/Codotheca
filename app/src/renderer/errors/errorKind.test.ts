import { describe, expect, it } from 'vitest';
import { ERROR_CODES } from '../../generated/protocol';
import { rowFixture } from '../project/testFixtures';
import {
  ERROR_ACTION_LABEL,
  explainErrorKind,
  lastTriedLine,
  neverSucceeded,
  TRY_AGAIN_LABEL,
} from './errorKind';

describe('the never-succeeded state', () => {
  it('is an error with no observation ever recorded', () => {
    expect(
      neverSucceeded(
        rowFixture({
          errorKind: 'PERMISSION_DENIED',
          refstateObservedAt: null,
          worktreeObservedAt: null,
        }),
      ),
    ).toBe(true);
  });

  it('is not the ordinary case: no error means nothing to explain', () => {
    expect(neverSucceeded(rowFixture())).toBe(false);
    expect(neverSucceeded(rowFixture({ refstateObservedAt: null, worktreeObservedAt: null }))).toBe(
      false,
    );
  });

  it('is not stale-but-once-known — either observation is enough to have been read', () => {
    expect(
      neverSucceeded(
        rowFixture({
          errorKind: 'STORE_OFFLINE',
          refstateObservedAt: 1_700_000_000,
          worktreeObservedAt: null,
        }),
      ),
    ).toBe(false);
    expect(
      neverSucceeded(
        rowFixture({
          errorKind: 'STORE_OFFLINE',
          refstateObservedAt: null,
          worktreeObservedAt: 1_700_000_000,
        }),
      ),
    ).toBe(false);
  });

  it('reads three columns and no others, so a caller may pass any row shape carrying them', () => {
    expect(
      neverSucceeded({
        errorKind: 'REPO_UNREADABLE',
        refstateObservedAt: null,
        worktreeObservedAt: null,
      }),
    ).toBe(true);
  });
});

describe('the copy boundary', () => {
  // The generated list rather than a second one written here: a hand-copied enum is this
  // project's dominant defect class, and reading the generated array means a code added to
  // `protocol.json` reaches this loop without anyone remembering to add it.
  it('reads the generated enum, or every loop below is vacuous', () => {
    expect(ERROR_CODES.length).toBe(12);
  });

  it('answers for every member of the closed enum', () => {
    for (const code of ERROR_CODES) {
      expect(explainErrorKind(code).placement).toMatch(/^(project|startup|never)$/);
    }
  });

  it('places the six §11.1 kinds on the project, with a badge and prose each', () => {
    const project = [
      'PERMISSION_DENIED',
      'UNTRUSTED_REPO',
      'REPO_UNREADABLE',
      'PATH_GONE',
      'STORE_OFFLINE',
      'BUDGET_EXCEEDED',
    ] as const;
    for (const code of project) {
      const e = explainErrorKind(code);
      expect(e.placement).toBe('project');
      expect(e.badge).not.toBeNull();
      expect(e.prose).not.toBeNull();
    }
    expect(explainErrorKind('PERMISSION_DENIED').badge).toBe('NOT INDEXED');
    expect(explainErrorKind('UNTRUSTED_REPO').badge).toBe('NOT TRUSTED');
    expect(explainErrorKind('PATH_GONE').badge).toBe('MISSING');
    expect(explainErrorKind('STORE_OFFLINE').badge).toBe('OFFLINE');
  });

  it('sends the two git kinds to startup with no badge — they set on every project at once', () => {
    for (const code of ['GIT_MISSING', 'GIT_TOO_OLD'] as const) {
      expect(explainErrorKind(code)).toEqual({
        placement: 'startup',
        badge: null,
        prose: null,
        action: null,
      });
    }
  });

  it('keeps the four app-level codes off project.error_kind entirely', () => {
    for (const code of ['CORE_RESTARTED', 'PROTOCOL', 'INTERNAL', 'PROJECT_MERGED'] as const) {
      expect(explainErrorKind(code).placement).toBe('never');
      expect(explainErrorKind(code).badge).toBeNull();
    }
  });

  it('offers exactly the two controls §11.1 draws, and no third', () => {
    expect(explainErrorKind('UNTRUSTED_REPO').action).toBe('trust');
    expect(explainErrorKind('PATH_GONE').action).toBe('relocate');
    for (const code of [
      'PERMISSION_DENIED',
      'REPO_UNREADABLE',
      'STORE_OFFLINE',
      'BUDGET_EXCEEDED',
    ] as const) {
      expect(explainErrorKind(code).action).toBeNull();
    }
    expect(Object.values(ERROR_ACTION_LABEL)).toEqual(['TRUST THIS REPOSITORY', 'RELOCATE']);
  });

  it('never emits a destructive token, and never a merge (§17, criterion 44)', () => {
    const strings = ERROR_CODES.flatMap((c) => {
      const e = explainErrorKind(c);
      return [e.badge, e.prose].filter((s): s is string => s !== null);
    }).concat(Object.values(ERROR_ACTION_LABEL), TRY_AGAIN_LABEL);
    expect(strings.length).toBeGreaterThan(12);
    for (const s of strings) {
      expect(s).not.toMatch(/forget|delete|remove|uninstall|merge/i);
    }
  });
});

describe('ruling A: STORE_OFFLINE is never-succeeded, not §8.5.2 presence', () => {
  it('does not claim a frozen condition for a repository that was never read', () => {
    const prose = explainErrorKind('STORE_OFFLINE').prose ?? '';
    expect(prose).toContain('never been read');
    // §8.5.2's presence sentence describes a measurement this project does not have.
    expect(prose).not.toMatch(/frozen|rotting|decaying/i);
  });
});

describe('LAST TRIED', () => {
  it('renders nothing at all when nothing was ever tried — never a zero and never a date', () => {
    expect(lastTriedLine(null, 10_000)).toBeNull();
  });

  it('states an age from error_at and nothing else', () => {
    expect(lastTriedLine(10_000 - 3 * 86_400, 10_000 + 3 * 86_400)).toBe('LAST TRIED 6d AGO');
    expect(lastTriedLine(9_990, 10_000)).toBe('LAST TRIED JUST NOW');
  });
});
